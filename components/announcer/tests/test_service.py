import hashlib
import json
import tempfile
import time
import unittest
import urllib.error
import urllib.request

from announcer.service import Service


class AnnouncementContractTest(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.config = {
            "data_dir": self.directory.name,
            "api_bind": "127.0.0.1", "api_port": 0,
            "media_bind": "127.0.0.1", "media_port": 0,
            "callers": {
                "door": {"token_sha256": hashlib.sha256(b"door-token").hexdigest(), "topics": ["door.open"]},
                "other": {"token_sha256": hashlib.sha256(b"other-token").hexdigest(), "topics": ["door.open"]},
            },
            "outputs": {"speaker": {"topics": ["door.open"], "source_ip": "127.0.0.1"}},
        }
        self.start()

    def start(self):
        self.service = Service(self.config)
        self.service.start()
        self.base = f"http://127.0.0.1:{self.service.api.server_port}"

    def tearDown(self):
        self.service.close()
        self.directory.cleanup()

    def request(self, method, path="/v1/announcements", body=None, token="door-token", key="event-1"):
        headers = {"Authorization": f"Bearer {token}", "Idempotency-Key": key, "Content-Type": "application/json"}
        data = None if body is None else json.dumps(body).encode()
        request = urllib.request.Request(self.base + path, data=data, headers=headers, method=method)
        try:
            response = urllib.request.urlopen(request, timeout=3)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            return response.status, json.load(response)

    def test_authenticated_topic_admission_and_caller_scoped_idempotency_survive_restart(self):
        body = {"topic": "door.open", "text": "The side door opened.", "ttl_seconds": 30}
        self.assertEqual(self.request("POST", body=body, token="wrong")[0], 401)
        self.assertEqual(self.request("POST", body={**body, "topic": "alarm"})[0], 403)
        status, accepted = self.request("POST", body=body)
        self.assertEqual(status, 202)
        self.assertEqual(accepted["status"], "accepted")
        path = "/v1/announcements/" + accepted["id"]
        self.assertEqual(self.request("GET", path, token="other-token")[0], 404)
        self.assertEqual(self.request("POST", body={**body, "text": "Another event."})[0], 409)
        self.service.close()
        self.start()
        status, retried = self.request("POST", body=body)
        self.assertEqual(status, 202)
        self.assertEqual(retried["id"], accepted["id"])
        self.assertEqual(retried["expires_at"], accepted["expires_at"])
        self.assertNotIn("text", self.request("GET", path)[1])
        status, separate = self.request("POST", body=body, token="other-token")
        self.assertEqual(status, 202)
        self.assertNotEqual(separate["id"], accepted["id"])

    def test_queue_is_bounded_and_expired_retry_does_not_extend_deadline(self):
        body = {"topic": "door.open", "text": "Door opened.", "ttl_seconds": 1}
        for index in range(8):
            status, accepted = self.request("POST", body=body, key=f"event-{index}")
            self.assertEqual(status, 202)
        self.assertEqual(self.request("POST", body=body, key="overflow")[0], 429)
        self.assertEqual(self.request("POST", body={**body, "volume": 1}, key="unsafe")[0], 400)
        time.sleep(1.05)
        status, retried = self.request("POST", body=body, key="event-7")
        self.assertEqual(retried["status"], "expired")
        self.assertEqual(retried["expires_at"], accepted["expires_at"])
        self.assertEqual(retried["outputs"], {"speaker": "expired"})
        self.assertEqual(self.request("POST", body=body, token="other-token", key="new")[0], 202)

    def test_text_limit_counts_whitespace_and_lifetime_requires_integer(self):
        body = {"topic": "door.open", "text": "x" + " " * 240, "ttl_seconds": 30}
        self.assertEqual(self.request("POST", body=body)[0], 400)
        body["text"] = "Door opened."
        for invalid in (True, 0, 31, 1.5, "30"):
            self.assertEqual(self.request("POST", body={**body, "ttl_seconds": invalid})[0], 400)
        self.assertEqual(self.request("POST", body={**body, "text": "x" * 240})[0], 202)


if __name__ == "__main__":
    unittest.main()
