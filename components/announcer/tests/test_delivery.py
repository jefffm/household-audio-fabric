import http.client
import json
import os
import queue
import select
import socket
import socketserver
import subprocess
import sys
import threading
import time
import unittest
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import paho.mqtt.client as mqtt

import test_service


@unittest.skipUnless(os.environ.get("ANNOUNCER_TEST_MODEL"), "set ANNOUNCER_TEST_MODEL for broker/speech integration")
class DeliveryContractTest(unittest.TestCase):
    request = test_service.AnnouncementContractTest.request
    start = test_service.AnnouncementContractTest.start

    @classmethod
    def setUpClass(cls):
        import tempfile
        cls.broker_files = tempfile.TemporaryDirectory()
        root = Path(cls.broker_files.name)
        root.chmod(0o755)
        (root / "mosquitto.conf").write_text("listener 1883\nallow_anonymous true\nacl_file /test/acl\n")
        (root / "acl").write_text("topic readwrite test/allowed\n")
        image = "docker.io/library/eclipse-mosquitto@sha256:212f89e1eaeb2c322d6441b64396e3346026674db8fa9c27beac293405c32b3c"
        cls.container = subprocess.check_output(["podman", "run", "--rm", "-d", "-p", "127.0.0.1::1883", "-v", f"{root}:/test:ro", image, "mosquitto", "-c", "/test/mosquitto.conf"], text=True).strip()
        cls.port = int(subprocess.check_output(["podman", "port", cls.container, "1883"], text=True).rsplit(":", 1)[1])
        for _ in range(50):
            try:
                with socket.create_connection(("127.0.0.1", cls.port), timeout=0.1):
                    return
            except OSError:
                time.sleep(0.1)
        raise RuntimeError("test broker unavailable")

    @classmethod
    def tearDownClass(cls):
        subprocess.run(["podman", "stop", "--time", "1", cls.container], check=True, stdout=subprocess.DEVNULL)
        cls.broker_files.cleanup()

    def setUp(self):
        test_service.AnnouncementContractTest.setUp(self)
        self.service.close()
        self.messages = queue.Queue()
        self.subscriber = mqtt.Client(mqtt.CallbackAPIVersion.VERSION2, protocol=mqtt.MQTTv5)
        subscribed = threading.Event()
        self.subscriber.on_connect = lambda client, *args: client.subscribe("test/allowed", qos=1)
        self.subscriber.on_subscribe = lambda *args: subscribed.set()
        self.subscriber.on_message = lambda client, userdata, message: self.messages.put(message)
        self.subscriber.connect("127.0.0.1", self.port)
        self.subscriber.loop_start()
        self.assertTrue(subscribed.wait(3))
        self.idle = True
        owner = self

        class Snapcast(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_POST(self):
                self.rfile.read(int(self.headers["Content-Length"]))
                response = {"result": {"server": {
                    "groups": [{"stream_id": "music", "clients": [{"id": "test-speaker", "connected": True}]}],
                    "streams": [{"id": "music", "status": "idle" if owner.idle else "playing"}],
                }}}
                data = json.dumps(response).encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

        self.snapcast = ThreadingHTTPServer(("127.0.0.1", 0), Snapcast)
        self.snapcast_thread = threading.Thread(target=self.snapcast.serve_forever)
        self.snapcast_thread.start()
        self.config.update({
            "model": os.environ["ANNOUNCER_TEST_MODEL"],
            "mqtt": {"host": "127.0.0.1", "port": self.port},
            "snapcast_url": f"http://127.0.0.1:{self.snapcast.server_port}/jsonrpc",
            "media_base_url": "http://127.0.0.1:1",
        })
        self.config["outputs"]["speaker"].update({"snapcast_client_id": "test-speaker", "notify_topic": "test/allowed", "volume": 0.1})
        self.start()
        self.media_base = f"http://127.0.0.1:{self.service.media.server_port}"

    def tearDown(self):
        self.service.close()
        self.subscriber.disconnect()
        self.subscriber.loop_stop()
        self.snapcast.shutdown()
        self.snapcast.server_close()
        self.snapcast_thread.join()
        self.directory.cleanup()

    def await_status(self, job, expected):
        for _ in range(200):
            status, result = self.request("GET", "/v1/announcements/" + job)
            if result["status"] == expected:
                return result
            time.sleep(0.05)
        self.fail(f"expected {expected}, got {result}")

    def test_http_to_speech_to_nonretained_mqtt_and_private_range_media(self):
        status, job = self.request("POST", body={"topic": "door.open", "text": "The side door opened.", "ttl_seconds": 10})
        self.assertEqual(status, 202)
        message = self.messages.get(timeout=10)
        self.assertFalse(message.retain)
        self.assertGreater(message.properties.MessageExpiryInterval, 0)
        self.assertLessEqual(message.properties.MessageExpiryInterval, 10)
        payload = json.loads(message.payload)
        self.assertEqual(payload["volume"], 0.1)
        self.assertFalse(payload["wake_screen"])
        self.assertEqual(set(payload), {"sound", "volume", "wake_screen"})
        self.await_status(job["id"], "published_unconfirmed")
        path = urllib.parse.urlsplit(payload["sound"]).path
        foreign = http.client.HTTPConnection("127.0.0.1", self.service.media.server_port, source_address=("127.0.0.2", 0))
        try:
            foreign.request("GET", path, headers={"X-Forwarded-For": "127.0.0.1"})
            self.assertEqual(foreign.getresponse().status, 404)
        finally:
            foreign.close()
        with self.assertRaises(urllib.error.HTTPError) as invalid_range:
            urllib.request.urlopen(urllib.request.Request(self.media_base + path, headers={"Range": "bytes=0-1,4-5"}))
        self.assertEqual(invalid_range.exception.code, 416)
        with urllib.request.urlopen(urllib.request.Request(self.media_base + path, headers={"Range": "bytes=0-43"})) as response:
            self.assertEqual(response.status, 206)
            header = response.read()
            self.assertEqual(header[:4], b"RIFF")
            self.assertEqual(len(header), 44)
        # A partial header fetch does not prove full media delivery.
        self.assertEqual(self.request("GET", "/v1/announcements/" + job["id"])[1]["status"], "published_unconfirmed")
        with urllib.request.urlopen(self.media_base + path) as response:
            self.assertGreater(len(response.read()), 44000)
        self.await_status(job["id"], "media_fetched_unconfirmed")
        with self.assertRaises(urllib.error.HTTPError) as rejected:
            urllib.request.urlopen(self.media_base + path.replace("speaker", "another"))
        self.assertEqual(rejected.exception.code, 404)
        self.subscriber.unsubscribe("test/allowed")
        self.subscriber.subscribe("test/allowed", qos=1)
        self.service.close()
        self.start()
        time.sleep(0.2)
        self.assertTrue(self.messages.empty(), "restart or retained subscription replayed an announcement")
        time.sleep(max(0, job["expires_at"] - time.time()) + 0.1)
        with self.assertRaises(urllib.error.HTTPError) as expired:
            urllib.request.urlopen(f"http://127.0.0.1:{self.service.media.server_port}" + path)
        self.assertEqual(expired.exception.code, 404)

    def test_busy_output_expires_without_publication(self):
        self.idle = False
        _, job = self.request("POST", body={"topic": "door.open", "text": "Door opened.", "ttl_seconds": 1})
        self.await_status(job["id"], "expired")
        self.idle = True
        time.sleep(0.3)
        self.assertTrue(self.messages.empty())

    def test_broker_acl_denial_is_not_reported_as_published(self):
        self.service.close()
        self.config["outputs"]["speaker"]["notify_topic"] = "test/forbidden"
        self.start()
        _, job = self.request("POST", body={"topic": "door.open", "text": "Door opened.", "ttl_seconds": 10})
        result = self.await_status(job["id"], "failed")
        self.assertEqual(result["outputs"], {"speaker": "failed"})
        self.assertTrue(self.messages.empty())

    def test_crash_after_delivery_without_puback_never_replays(self):
        broker_port = self.port

        class DropAcknowledgement(socketserver.BaseRequestHandler):
            def handle(self):
                with socket.create_connection(("127.0.0.1", broker_port), timeout=2) as upstream:
                    deadline = time.monotonic() + 6
                    while time.monotonic() < deadline:
                        ready, _, _ = select.select([self.request, upstream], [], [], 0.1)
                        for source in ready:
                            data = source.recv(8192)
                            if not data:
                                return
                            # CONNACK forwards; the broker PUBACK is deliberately lost.
                            if source is upstream and data[0] >> 4 == 4:
                                continue
                            (upstream if source is self.request else self.request).sendall(data)

        proxy = socketserver.ThreadingTCPServer(("127.0.0.1", 0), DropAcknowledgement)
        proxy_thread = threading.Thread(target=proxy.serve_forever)
        proxy_thread.start()
        self.service.close()
        self.config["mqtt"]["port"] = proxy.server_address[1]
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            self.config["api_port"] = reservation.getsockname()[1]
        self.base = f"http://127.0.0.1:{self.config['api_port']}"
        config_path = Path(self.directory.name) / "config.json"
        config_path.write_text(json.dumps(self.config))
        process = None
        try:
            process = subprocess.Popen([sys.executable, "-m", "announcer.main"], env={**os.environ, "ANNOUNCER_CONFIG": str(config_path)}, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            for _ in range(100):
                self.assertIsNone(process.poll(), "service process did not start")
                try:
                    with urllib.request.urlopen(self.base + "/healthz", timeout=0.1):
                        break
                except OSError:
                    time.sleep(0.05)
            _, job = self.request("POST", body={"topic": "door.open", "text": "Door opened.", "ttl_seconds": 10})
            self.messages.get(timeout=10)
            process.kill()
            process.wait(timeout=3)
            self.start()
            self.await_status(job["id"], "uncertain")
            time.sleep(0.5)
            self.assertTrue(self.messages.empty())
        finally:
            if process and process.poll() is None:
                process.kill()
                process.wait(timeout=3)
            proxy.shutdown()
            proxy.server_close()
            proxy_thread.join()


if __name__ == "__main__":
    unittest.main()
