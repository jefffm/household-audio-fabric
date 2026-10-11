import base64
import hashlib
import io
import os
import time
import unittest
import wave

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
