"""Configured single-replica entry point; secrets never appear in diagnostics."""

import ipaddress
import json
import math
import os
import re
import signal
import threading
from pathlib import Path
from urllib.parse import urlsplit

from .service import Service


def require(condition):
    if not condition:
        raise ValueError("invalid configuration")


def main():
    os.umask(0o077)
    try:
        config = json.loads(Path(os.environ["ANNOUNCER_CONFIG"]).read_text())
        for name in ("api_bind", "media_bind"):
            ipaddress.IPv4Address(config[name])
        for name in ("api_port", "media_port"):
            require(type(config[name]) is int and 0 <= config[name] <= 65535)
        require(1 <= len(config["callers"]) <= 32 and 1 <= len(config["outputs"]) <= 8)
        for caller in config["callers"].values():
            require(re.fullmatch(r"[a-f0-9]{64}", caller["token_sha256"]))
            require(isinstance(caller["topics"], list) and caller["topics"] and all(isinstance(t, str) and t for t in caller["topics"]))
        for name, output in config["outputs"].items():
            require(re.fullmatch(r"[A-Za-z0-9_-]{1,64}", name))
            ipaddress.IPv4Address(output["source_ip"])
            require(math.isfinite(output["volume"]) and 0 <= output["volume"] <= 0.1)
            require(isinstance(output["topics"], list) and output["topics"] and output["snapcast_client_id"])
            require(output["notify_topic"] and not any(c in output["notify_topic"] for c in "+#\x00"))
        for name in ("media_base_url", "snapcast_url"):
            url = urlsplit(config[name])
            require(url.scheme == "http" and url.hostname and not (url.username or url.password or url.query or url.fragment))
        require(not urlsplit(config["media_base_url"]).path)
        ipaddress.IPv4Address(config["mqtt"]["host"])
        if "username" in config["mqtt"]:
            require(Path(config["mqtt"]["password_file"]).is_file())
        require(Path(config["model"]).is_file() and Path(config["model"] + ".json").is_file())
    except (ValueError, TypeError, KeyError, OSError):
        raise SystemExit("invalid_announcement_configuration") from None
    stopped = threading.Event()
    signal.signal(signal.SIGTERM, lambda *args: stopped.set())
    signal.signal(signal.SIGINT, lambda *args: stopped.set())
    service = Service(config)
    service.start()
    print("announcement_service_ready", flush=True)
    try:
        stopped.wait()
    finally:
        service.close()


if __name__ == "__main__":
    main()
