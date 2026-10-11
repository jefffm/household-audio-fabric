"""Authenticated announcement admission and durable, single-replica lifecycle."""

import array
import fcntl
import hashlib
import hmac
import json
import re
import sqlite3
import subprocess
import sys
import threading
import time
import uuid
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from . import media, portal


class HttpError(Exception):
    def __init__(self, status, code):
        self.status = status
        self.code = code


class BoundedServer(ThreadingHTTPServer):
    daemon_threads = False

    def __init__(self, address, handler):
        self.slots = threading.BoundedSemaphore(8)
        super().__init__(address, handler)

    def process_request(self, request, address):
        request.settimeout(2)
        if not self.slots.acquire(blocking=False):
            self.shutdown_request(request)
            return
        super().process_request(request, address)

    def process_request_thread(self, request, address):
        try:
            super().process_request_thread(request, address)
        finally:
            self.slots.release()

    def handle_error(self, request, address):
        # Never log request bodies, credentials, paths or source URLs.
        print("http_request_failed", flush=True)


class Service:
    def __init__(self, config):
        self.config = config
        self.lock = threading.RLock()
        self.directory = Path(config["data_dir"])
        self.directory.mkdir(parents=True, exist_ok=True)
        self.process_lock = (self.directory / "lock").open("a")
        fcntl.flock(self.process_lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        self.stopping = threading.Event()
        self.closed = False
        self.db = sqlite3.connect(self.directory / "announcements.sqlite", check_same_thread=False)
        self.db.row_factory = sqlite3.Row
        self.db.executescript("""
            PRAGMA journal_mode=WAL;
            PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS jobs (
                id TEXT PRIMARY KEY, caller TEXT NOT NULL, key TEXT NOT NULL,
                fingerprint TEXT NOT NULL, topic TEXT NOT NULL, text TEXT NOT NULL,
                created REAL NOT NULL, expires REAL NOT NULL, status TEXT NOT NULL,
                UNIQUE(caller, key)
            );
            CREATE TABLE IF NOT EXISTS deliveries (
                job TEXT NOT NULL REFERENCES jobs(id), output TEXT NOT NULL,
                status TEXT NOT NULL, requests INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(job, output)
            );
        """)
        with self.db:
            self.db.execute("UPDATE deliveries SET status='uncertain' WHERE status='dispatching'")
            self.db.execute("UPDATE deliveries SET status='failed' WHERE status='accepted' AND job IN (SELECT id FROM jobs WHERE status='synthesizing')")
            for row in self.db.execute("SELECT id FROM jobs WHERE status='synthesizing'").fetchall():
                self.refresh(row["id"])
            self.cleanup()
        service = self

        class Api(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def respond(self, status, body):
                encoded = json.dumps(body).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(encoded)))
                self.send_header("Cache-Control", "no-store")
                self.end_headers()
                self.wfile.write(encoded)

            def do_GET(self):
                self.handle_api()

            def do_POST(self):
                self.handle_api()

            def handle_api(self):
                try:
                    if self.command == "GET" and self.path == "/healthz":
                        alive = not hasattr(service, "worker") or service.worker.is_alive()
                        self.respond(200 if alive else 503, {"status": "ok" if alive else "failed"})
                        return
                    caller = service.authenticate(self.headers.get("Authorization", ""))
                    if self.command == "POST" and self.path == "/v1/announcements":
                        if self.headers.get_content_type() != "application/json":
                            raise HttpError(415, "json_required")
                        length = self.headers.get("Content-Length", "")
                        if self.headers.get("Transfer-Encoding") or not length.isdecimal():
                            raise HttpError(400, "content_length_required")
                        if not 0 < int(length) <= 2048:
                            raise HttpError(413, "request_too_large")
                        body = json.loads(self.rfile.read(int(length)))
                        result = service.accept(caller, self.headers.get("Idempotency-Key", ""), body)
                        self.respond(202, result)
                    elif self.command == "GET" and re.fullmatch(r"/v1/announcements/[a-f0-9]{32}", self.path):
                        self.respond(200, service.status(caller, self.path.rsplit("/", 1)[1]))
                    else:
                        raise HttpError(404, "not_found")
                except HttpError as error:
                    self.respond(error.status, {"error": error.code})
                except (ValueError, UnicodeError):
                    self.respond(400, {"error": "invalid_request"})

        self.api = BoundedServer((config["api_bind"], config["api_port"]), Api)
        self.media = BoundedServer((config["media_bind"], config["media_port"]), media.handler(self))
        self.threads = []

    def authenticate(self, authorization):
        if not authorization.startswith("Bearer "):
            raise HttpError(401, "unauthorized")
        digest = hashlib.sha256(authorization[7:].encode()).hexdigest()
        for name, caller in self.config["callers"].items():
            if hmac.compare_digest(digest, caller["token_sha256"]):
                return name
        raise HttpError(401, "unauthorized")

    def accept(self, caller, key, body):
        if not isinstance(body, dict) or set(body) != {"topic", "text", "ttl_seconds"}:
            raise HttpError(400, "invalid_request")
        topic, text, ttl = body["topic"], body["text"], body["ttl_seconds"]
        if not re.fullmatch(r"[A-Za-z0-9_.:-]{1,128}", key):
            raise HttpError(400, "invalid_idempotency_key")
        if not isinstance(topic, str) or topic not in self.config["callers"][caller]["topics"]:
            raise HttpError(403, "topic_forbidden")
        if not isinstance(text, str) or not text.strip() or len(text) > 240 or any(ord(c) < 32 for c in text):
            raise HttpError(400, "invalid_text")
        if type(ttl) is not int or not 1 <= ttl <= 30:
            raise HttpError(400, "invalid_lifetime")
        fingerprint = hashlib.sha256(json.dumps(body, sort_keys=True).encode()).hexdigest()
        with self.lock, self.db:
            self.cleanup()
            old = self.db.execute("SELECT * FROM jobs WHERE caller=? AND key=?", (caller, key)).fetchone()
            if old:
                if old["fingerprint"] != fingerprint:
                    raise HttpError(409, "idempotency_conflict")
                return self.status(caller, old["id"])
            outputs = [name for name, output in self.config["outputs"].items() if topic in output["topics"]]
            if not outputs:
                raise HttpError(422, "no_subscribers")
            now = time.time()
            queued = self.db.execute("SELECT count(*) FROM jobs WHERE status='accepted'").fetchone()[0]
            recent = self.db.execute("SELECT count(*) FROM jobs WHERE caller=? AND created>?", (caller, now - 60)).fetchone()[0]
            total = self.db.execute("SELECT count(*) FROM jobs").fetchone()[0]
            if queued >= 8 or recent >= 8 or total >= 10000:
                raise HttpError(429, "capacity_exceeded")
            job = uuid.uuid4().hex
            self.db.execute("INSERT INTO jobs VALUES (?,?,?,?,?,?,?,?,?)", (
                job, caller, key, fingerprint, topic, text, now, now + ttl, "accepted"))
            self.db.executemany("INSERT INTO deliveries (job,output,status) VALUES (?,?,?)", [(job, output, "accepted") for output in outputs])
        return self.status(caller, job)

    def cleanup(self):
        now = time.time()
        self.db.execute("UPDATE deliveries SET status='expired' WHERE status='accepted' AND job IN (SELECT id FROM jobs WHERE expires<=?)", (now,))
        self.db.execute("UPDATE jobs SET status='expired',text='' WHERE status='accepted' AND expires<=?", (now,))
        self.db.execute("DELETE FROM deliveries WHERE job IN (SELECT id FROM jobs WHERE created<?)", (now - 86400,))
        self.db.execute("DELETE FROM jobs WHERE created<?", (now - 86400,))
        for path in self.directory.glob("*.wav"):
            row = self.db.execute("SELECT expires FROM jobs WHERE id=?", (path.stem,)).fetchone()
            if not row or row["expires"] <= now:
                path.unlink(missing_ok=True)

    def status(self, caller, job):
        with self.lock, self.db:
            self.cleanup()
            row = self.db.execute("SELECT * FROM jobs WHERE caller=? AND id=?", (caller, job)).fetchone()
            if not row:
                raise HttpError(404, "not_found")
            deliveries = self.db.execute("SELECT output,status FROM deliveries WHERE job=?", (job,)).fetchall()
            return {"id": row["id"], "status": row["status"], "expires_at": row["expires"],
                    "outputs": {d["output"]: d["status"] for d in deliveries}}

    def refresh(self, job):
        states = {row[0] for row in self.db.execute("SELECT status FROM deliveries WHERE job=?", (job,))}
        priority = ["uncertain", "dispatching", "accepted", "failed", "expired", "published_unconfirmed", "media_fetched_unconfirmed"]
        status = next(value for value in priority if value in states)
        if status in {"dispatching", "accepted"}:
            status = "synthesizing"
        self.db.execute("UPDATE jobs SET status=?,text='' WHERE id=?", (status, job))

    def media_data(self, job, name, source, head):
        output = self.config["outputs"].get(name)
        if not output or output["source_ip"] != source:
            return None
        with self.lock, self.db:
            row = self.db.execute("SELECT d.status,d.requests,j.expires,j.topic FROM deliveries d JOIN jobs j ON d.job=j.id WHERE d.job=? AND d.output=?", (job, name)).fetchone()
            if not row or row["expires"] <= time.time() or row["topic"] not in output["topics"]:
                return None
            if row["status"] not in {"dispatching", "uncertain", "published_unconfirmed", "media_fetched_unconfirmed"} or row["requests"] >= 16:
                return None
            path = self.directory / f"{job}.wav"
            if not path.is_file() or path.stat().st_size > 960044:
                return None
            if not head:
                self.db.execute("UPDATE deliveries SET requests=requests+1 WHERE job=? AND output=?", (job, name))
            return path.read_bytes()

    def fetched(self, job, output):
        with self.lock, self.db:
            self.db.execute("UPDATE deliveries SET status='media_fetched_unconfirmed' WHERE job=? AND output=?", (job, output))
            self.refresh(job)

    def work(self):
        while not self.stopping.wait(0.1):
            with self.lock, self.db:
                self.cleanup()
                # Conservatively hold the lane through the last possible download
                # plus the maximum clip length, including across a restart.
                cooling = self.db.execute("SELECT 1 FROM jobs j JOIN deliveries d ON j.id=d.job WHERE j.expires+12>? AND d.status IN ('dispatching','uncertain','published_unconfirmed','media_fetched_unconfirmed') LIMIT 1", (time.time(),)).fetchone()
                row = self.db.execute("SELECT * FROM jobs WHERE status='accepted' ORDER BY created LIMIT 1").fetchone()
                if cooling or not row:
                    continue
                outputs = [d[0] for d in self.db.execute("SELECT output FROM deliveries WHERE job=?", (row["id"],))]
                authorized = row["topic"] in self.config["callers"].get(row["caller"], {}).get("topics", [])
                authorized = authorized and all(row["topic"] in self.config["outputs"].get(name, {}).get("topics", []) for name in outputs)
                if not authorized:
                    self.db.execute("UPDATE deliveries SET status='failed' WHERE job=?", (row["id"],))
                    self.refresh(row["id"])
                    continue
            if not portal.idle(self.config, outputs):
                continue
            with self.lock, self.db:
                if row["expires"] <= time.time() or self.stopping.is_set():
                    continue
                self.db.execute("UPDATE jobs SET status='synthesizing' WHERE id=?", (row["id"],))
            self.deliver(row, outputs)

    def deliver(self, row, outputs):
        job = row["id"]
        path = self.directory / f"{job}.wav"
        try:
            subprocess.run([sys.executable, "-m", "announcer.synthesize", "--model", self.config["model"], "--output", str(path), "--max-seconds", "10"],
                           input=row["text"].encode(), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                           timeout=max(0.01, min(15, row["expires"] - time.time())), check=True)
            with wave.open(str(path)) as wav:
                if wav.getnchannels() != 1 or wav.getsampwidth() != 2 or not 8000 <= wav.getframerate() <= 48000 or not 0 < wav.getnframes() <= 10 * wav.getframerate():
                    raise ValueError("invalid clip")
                samples = array.array("h", wav.readframes(wav.getnframes()))
                if sys.byteorder != "little":
                    samples.byteswap()
                if max(abs(sample) for sample in samples) > 3932:
                    raise ValueError("unsafe peak")
            for name in outputs:
                state = "failed"
                if row["expires"] <= time.time():
                    state = "expired"
                elif not self.stopping.is_set() and portal.idle(self.config, [name]):
                    with self.lock, self.db:
                        # Commit dispatch intent before any MQTT bytes can leave.
                        self.db.execute("UPDATE deliveries SET status='dispatching' WHERE job=? AND output=?", (job, name))
                    url = f"{self.config['media_base_url']}/media/{job}/{name}.wav"
                    state = portal.publish(self.config, self.config["outputs"][name], url, row["expires"])
                with self.lock, self.db:
                    self.db.execute("UPDATE deliveries SET status=? WHERE job=? AND output=? AND status!='media_fetched_unconfirmed'", (state, job, name))
        except (OSError, ValueError, wave.Error, subprocess.SubprocessError):
            with self.lock, self.db:
                state = "expired" if row["expires"] <= time.time() else "failed"
                self.db.execute("UPDATE deliveries SET status=? WHERE job=? AND status='accepted'", (state, job))
                self.db.execute("UPDATE deliveries SET status='uncertain' WHERE job=? AND status='dispatching'", (job,))
        finally:
            with self.lock, self.db:
                self.refresh(job)

    def start(self):
        if "mqtt" in self.config:
            self.worker = threading.Thread(target=self.work)
            self.threads.append(self.worker)
            self.worker.start()
        for server in (self.api, self.media):
            thread = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.05})
            self.threads.append(thread)
            thread.start()

    def close(self):
        if self.closed:
            return
        self.closed = True
        self.stopping.set()
        for server in (self.api, self.media):
            server.shutdown()
            server.server_close()
        for thread in self.threads:
            thread.join()
        self.db.close()
        self.process_lock.close()
