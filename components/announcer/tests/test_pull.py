import base64
import hashlib
import io
import json
import os
import socket
import time
import unittest
import urllib.error
import urllib.request
import wave
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from urllib.parse import urlparse

import test_service


@unittest.skipUnless(os.environ.get("ANNOUNCER_TEST_MODEL"), "set ANNOUNCER_TEST_MODEL for native delivery")
class NativeDeliveryTest(unittest.TestCase):
    request = test_service.AnnouncementContractTest.request
    start = test_service.AnnouncementContractTest.start

    def setUp(self):
        test_service.AnnouncementContractTest.setUp(self)
        self.service.close()
        self.config["model"] = os.environ["ANNOUNCER_TEST_MODEL"]
        self.config["outputs"] = {
            "portal": {"transport": "home_today", "topics": ["door.open"],
                       "token_sha256": hashlib.sha256(b"output-token").hexdigest()},
            "other": {"transport": "home_today", "topics": ["other.topic"],
                      "token_sha256": hashlib.sha256(b"other-output-token").hexdigest()},
        }
        self.start()

    def tearDown(self):
        test_service.AnnouncementContractTest.tearDown(self)

    def next(self, output="portal", token="output-token"):
        return self.request("POST", f"/v1/outputs/{output}/next", token=token)

    def accept(self, ttl=30, key="event-1"):
        status, job = self.request("POST", body={"topic": "door.open", "text": "The side door opened.", "ttl_seconds": ttl}, key=key)
        self.assertEqual(status, 202)
        return job

    def claim(self):
        for _ in range(200):
            status, result = self.next()
            if result.get("announcement"):
                self.assertEqual(status, 200)
                return result["announcement"]
            time.sleep(0.05)
        self.fail("native speech was not ready")

    def wait_next(self, seconds=20, output="portal", token="output-token"):
        request = urllib.request.Request(self.base + f"/v1/outputs/{output}/next", data=b"", method="POST",
                                         headers={"Authorization": f"Bearer {token}", "Prefer": f"wait={seconds}"})
        with urllib.request.urlopen(request, timeout=25) as response:
            return json.load(response)["announcement"], int(response.headers["X-Announcement-Wait-Ms"])

    def test_long_poll_waits_without_blocking_admission_and_claims_fresh_audio_once(self):
        started = time.monotonic()
        empty, waited = self.wait_next(1)
        self.assertIsNone(empty)
        self.assertGreaterEqual(time.monotonic() - started, 0.95)
        self.assertGreaterEqual(waited, 950)
        self.assertLessEqual(waited, 1000)
        with ThreadPoolExecutor() as pool:
            pending = pool.submit(self.wait_next)
            time.sleep(0.15)
            self.assertFalse(pending.done(), "empty long poll must remain open")
            self.assertEqual(self.request("GET", "/healthz")[0], 200)
            self.assertEqual(self.next()[0], 409, "one waiter per output")
            self.assertEqual(self.next(token="wrong")[0], 401)
            job = self.accept()
            clip, waited = pending.result(timeout=15)
        self.assertEqual(clip["id"], job["id"])
        self.assertGreater(waited, 150)
        self.assertGreater(clip["remaining_ms"], 15000)
        self.assertLessEqual(clip["remaining_ms"], int((job["expires_at"] - time.time()) * 1000) + 100)
        self.assertIsNone(self.next()[1]["announcement"], "a long-poll claim must not replay")
        result = f"/v1/outputs/portal/{job['id']}/result"
        self.assertEqual(self.request("POST", result, {"status": "completed_unconfirmed"}, token="output-token")[0], 200)

    def test_abandoned_long_poll_releases_output_without_consuming_a_future_event(self):
        address = urlparse(self.base)
        with socket.create_connection((address.hostname, address.port), timeout=3) as client:
            client.sendall(b"POST /v1/outputs/portal/next HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer output-token\r\nPrefer: wait=20\r\nContent-Length: 0\r\n\r\n")
            time.sleep(0.15)
            self.assertEqual(self.next()[0], 409)
        time.sleep(0.4)
        self.assertIsNone(self.next()[1]["announcement"], "disconnect must release the output lease")
        job = self.accept()
        self.assertEqual(self.claim()["id"], job["id"])

    def test_shutdown_releases_idle_long_poll_promptly(self):
        with ThreadPoolExecutor() as pool:
            pending = pool.submit(self.wait_next)
            time.sleep(0.15)
            self.assertFalse(pending.done())
            started = time.monotonic()
            self.service.close()
            self.assertEqual(pending.result(timeout=2)[0], None)
            self.assertLess(time.monotonic() - started, 2)

    def test_wait_capacity_preserves_immediate_consumers_and_admission(self):
        self.service.close()
        for index in range(4):
            self.config["outputs"][f"waiting{index}"] = {
                "transport": "home_today", "topics": ["other.topic"],
                "token_sha256": hashlib.sha256(f"waiter-token-{index}".encode()).hexdigest()}
        self.start()
        with ThreadPoolExecutor() as pool:
            pending = [pool.submit(self.wait_next, 20, f"waiting{index}", f"waiter-token-{index}") for index in range(4)]
            try:
                time.sleep(0.2)
                self.assertTrue(all(not request.done() for request in pending))
                self.assertEqual(self.request("GET", "/healthz")[0], 200)
                self.assertEqual(self.next()[0], 200, "long-poll quota must not block other immediate consumers")
                with self.assertRaises(urllib.error.HTTPError) as error:
                    self.wait_next()
                self.assertEqual(error.exception.code, 409)
                self.accept()  # Admission must retain capacity while all wait slots are occupied.
            finally:
                self.service.close()
            for request in pending:
                self.assertIsNone(request.result(timeout=2)[0])

    def test_output_scoped_consume_once_and_ack_survive_restart(self):
        self.assertEqual(self.next(token="door-token")[0], 401)
        self.assertEqual(self.next(output="other")[0], 401)
        self.assertEqual(self.request("POST", body={}, token="output-token")[0], 401)
        job = self.accept()
        clip = self.claim()
        self.assertEqual(clip["id"], job["id"])
        self.assertEqual(clip["text"], "The side door opened.")
        self.assertGreater(clip["remaining_ms"], 0)
        self.assertLess(clip["remaining_ms"], 30000)
        with wave.open(io.BytesIO(base64.b64decode(clip["wav_base64"]))) as wav:
            self.assertEqual((wav.getnchannels(), wav.getsampwidth(), wav.getframerate()), (1, 2, 22050))
            self.assertLessEqual(wav.getnframes(), 220500)
        self.assertIsNone(self.next()[1]["announcement"])
        self.service.close()
        self.start()
        self.assertIsNone(self.next()[1]["announcement"], "restart must not replay a claimed response")
        path = f"/v1/outputs/portal/{job['id']}/result"
        self.assertEqual(self.request("POST", path, {"status": "played"}, token="output-token")[0], 400)
        self.assertEqual(self.request("POST", path, {"status": "completed_unconfirmed"}, token="output-token")[0], 200)
        self.assertEqual(self.request("POST", path, {"status": "completed_unconfirmed"}, token="output-token")[0], 200)
        self.assertEqual(self.request("POST", path, {"status": "failed"}, token="output-token")[0], 409)
        status = self.request("GET", "/v1/announcements/" + job["id"])[1]
        self.assertEqual(status["outputs"], {"portal": "completed_unconfirmed"})
        self.assertNotIn("text", status)
        second = self.accept(key="event-2")
        self.assertEqual(self.claim()["id"], second["id"], "terminal result must release the delivery lane")

    def test_native_delivery_does_not_wait_for_previous_playback_result(self):
        first = self.accept()
        self.assertEqual(self.claim()["id"], first["id"])
        # This phrase is warm, so playback of the first clip is the only possible hold.
        second = self.accept(key="interrupting-event")
        clip, _ = self.wait_next(1)
        self.assertIsNotNone(clip, "new native audio must be ready while the old clip is claimed")
        self.assertEqual(clip["id"], second["id"])
        for job, status in [(first, "interrupted"), (second, "completed_unconfirmed")]:
            path = f"/v1/outputs/portal/{job['id']}/result"
            self.assertEqual(self.request("POST", path, {"status": status}, token="output-token")[0], 200)
        self.assertEqual(self.request("GET", "/v1/announcements/" + first["id"])[1]["status"], "interrupted")
        self.assertIsNone(self.next()[1]["announcement"], "interruption must not replay either clip")

    def test_blocked_mixed_output_does_not_hold_later_native_delivery(self):
        self.service.close()
        self.config["callers"]["door"]["topics"].append("other.topic")
        self.config["outputs"]["legacy"] = {"topics": ["other.topic"], "source_ip": "127.0.0.1"}
        self.start()
        first = self.accept()
        self.assertEqual(self.claim()["id"], first["id"])
        status, blocked = self.request("POST", body={"topic": "other.topic", "text": "Legacy message.", "ttl_seconds": 30}, key="mixed")
        self.assertEqual(status, 202)
        newest = self.accept(key="new-native")
        clip, _ = self.wait_next(1)
        self.assertIsNotNone(clip, "blocked legacy output must not starve later native work")
        self.assertEqual(clip["id"], newest["id"])
        result = self.request("GET", "/v1/announcements/" + blocked["id"])[1]
        self.assertEqual(result["outputs"], {"other": "accepted", "legacy": "accepted"})

    def test_new_ready_native_audio_replaces_unclaimed_audio_per_output(self):
        self.service.close()
        self.config["outputs"]["other"]["topics"] = ["door.open"]
        self.start()
        first = self.accept()
        second = None
        for key in (None, "newest"):
            job = first if key is None else self.accept(key=key)
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                result = self.request("GET", "/v1/announcements/" + job["id"])[1]
                if result["status"] == "ready":
                    break
                time.sleep(0.05)
            self.assertEqual(result["status"], "ready")
            if key is None:
                self.assertEqual(self.next(output="other", token="other-output-token")[1]["announcement"]["id"], first["id"])
            else:
                second = job
        self.assertEqual(self.next()[1]["announcement"]["id"], second["id"])
        result = self.request("GET", "/v1/announcements/" + first["id"])[1]
        self.assertEqual(result["outputs"], {"portal": "interrupted", "other": "claimed"})
        # Superseding one output must not invalidate a different output's old report.
        path = f"/v1/outputs/other/{first['id']}/result"
        self.assertEqual(self.request("POST", path, {"status": "completed_unconfirmed"}, token="other-output-token")[0], 200)
        self.assertIsNone(self.next()[1]["announcement"])

    def test_repeated_speech_reuses_audio_not_delivery_and_cache_is_caller_scoped_and_ephemeral(self):
        self.service.close()
        model = Path(self.directory.name) / "voice.onnx"
        model.symlink_to(Path(self.config["model"]).resolve())
        Path(str(model) + ".json").symlink_to(Path(self.config["model"] + ".json").resolve())
        self.config["model"] = str(model)
        self.start()

        first = self.accept()
        first_clip = self.claim()
        result_path = f"/v1/outputs/portal/{first['id']}/result"
        self.assertEqual(self.request("POST", result_path, {"status": "completed_unconfirmed"}, token="output-token")[0], 200)
        model.unlink()  # Fresh inference is now unavailable; validated cached speech must still work.
        (Path(self.directory.name) / f"{first['id']}.wav").unlink()

        second = self.accept(key="event-2")
        second_clip = self.claim()
        self.assertNotEqual(second_clip["id"], first_clip["id"])
        self.assertEqual(second_clip["id"], second["id"])
        self.assertEqual(second_clip["wav_base64"], first_clip["wav_base64"])
        self.assertIsNone(self.next()[1]["announcement"], "cached audio must not replay a delivery")
        result_path = f"/v1/outputs/portal/{second['id']}/result"
        self.assertEqual(self.request("POST", result_path, {"status": "completed_unconfirmed"}, token="output-token")[0], 200)

        expired = self.accept(ttl=1, key="expired")
        time.sleep(max(0, expired["expires_at"] - time.time()) + 0.1)
        self.assertIsNone(self.next()[1]["announcement"], "cached speech must not extend event expiry")
        self.assertEqual(self.request("GET", "/v1/announcements/" + expired["id"])[1]["status"], "expired")

        def assert_inference_unavailable(text, token, key):
            status, job = self.request("POST", body={"topic": "door.open", "text": text, "ttl_seconds": 30}, token=token, key=key)
            self.assertEqual(status, 202)
            for _ in range(200):
                result = self.request("GET", "/v1/announcements/" + job["id"], token=token)[1]
                if result["status"] == "failed":
                    break
                time.sleep(0.05)
            self.assertEqual(result["status"], "failed")
            self.assertIsNone(self.next()[1]["announcement"])

        assert_inference_unavailable("The side door opened.", "other-token", "other-caller")
        assert_inference_unavailable("The side door closed.", "door-token", "different-text")
        self.service.close()
        self.start()
        assert_inference_unavailable("The side door opened.", "door-token", "after-restart")

    def test_broadcast_preserves_text_for_each_authorized_output(self):
        self.service.close()
        self.config["outputs"]["other"]["topics"] = ["door.open"]
        self.start()
        job = self.accept()
        first = self.claim()
        status, response = self.next(output="other", token="other-output-token")
        self.assertEqual(status, 200)
        self.assertEqual(response["announcement"]["text"], first["text"])
        self.assertEqual(response["announcement"]["id"], job["id"])
        self.assertIsNone(self.next()[1]["announcement"])

    def test_expired_unclaimed_delivery_is_not_returned_and_cannot_be_acked(self):
        job = self.accept(ttl=3)
        time.sleep(max(0, job["expires_at"] - time.time()) + 0.1)
        self.assertIsNone(self.next()[1]["announcement"])
        result = self.request("GET", "/v1/announcements/" + job["id"])[1]
        self.assertEqual(result["status"], "expired")
        path = f"/v1/outputs/portal/{job['id']}/result"
        self.assertEqual(self.request("POST", path, {"status": "completed_unconfirmed"}, token="output-token")[0], 409)
