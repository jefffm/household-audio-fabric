"""Read-only idle check and one-shot MQTT5 notification. Never reconnect/replay."""

import json
import math
import time
import urllib.request
import uuid
from pathlib import Path

import paho.mqtt.client as mqtt
from paho.mqtt.packettypes import PacketTypes
from paho.mqtt.properties import Properties


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args):
        return None


def idle(config, outputs):
    try:
        request = urllib.request.Request(config["snapcast_url"], data=b'{"id":1,"jsonrpc":"2.0","method":"Server.GetStatus"}', headers={"Content-Type": "application/json"})
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
        with opener.open(request, timeout=1) as response:
            server = json.loads(response.read(262145))["result"]["server"]
        streams = {stream["id"]: stream["status"] for stream in server["streams"]}
        connected_idle = {
            client["id"] for group in server["groups"]
            if streams.get(group["stream_id"]) == "idle"
            for client in group["clients"] if client["connected"] is True
        }
        return all(config["outputs"][name]["snapcast_client_id"] in connected_idle for name in outputs)
    except (OSError, ValueError, KeyError, TypeError):
        return False


def publish(config, output, url, expires):
    """Return failed before/after explicit refusal, uncertain after ambiguous send."""
    client = mqtt.Client(mqtt.CallbackAPIVersion.VERSION2, client_id="announce-" + uuid.uuid4().hex,
                         protocol=mqtt.MQTTv5, reconnect_on_failure=False)
    client.connect_timeout = 2
    client.max_queued_messages_set(1)
    client.max_inflight_messages_set(1)
    connection = []
    acknowledgements = []
    client.on_connect = lambda client, userdata, flags, reason, props: connection.append(reason.value)
    client.on_publish = lambda client, userdata, mid, reason, props: acknowledgements.append(reason.value)
    settings = config["mqtt"]
    if "username" in settings:
        client.username_pw_set(settings["username"], Path(settings["password_file"]).read_text().strip())
    sent = False
    deadline = min(expires, time.time() + 4)
    try:
        client.connect(settings["host"], settings.get("port", 1883), keepalive=10, clean_start=True)
        while not connection and time.time() < deadline:
            if client.loop(timeout=0.05) != mqtt.MQTT_ERR_SUCCESS:
                return "failed"
        remaining = math.floor(expires - time.time())
        if not connection or connection[0] != 0:
            return "failed"
        if remaining <= 0:
            return "expired"
        properties = Properties(PacketTypes.PUBLISH)
        properties.MessageExpiryInterval = remaining
        payload = json.dumps({"sound": url, "volume": min(0.1, max(0, output["volume"])), "wake_screen": False})
        # Mark ambiguous even if publish raises after putting bytes on the wire.
        sent = True
        info = client.publish(output["notify_topic"], payload, qos=1, retain=False, properties=properties)
        if info.rc != mqtt.MQTT_ERR_SUCCESS:
            return "uncertain"
        while not acknowledgements and time.time() < deadline:
            if client.loop(timeout=0.05) != mqtt.MQTT_ERR_SUCCESS:
                return "uncertain"
        if not acknowledgements:
            return "uncertain"
        # 0x10 means no matching subscribers, not delivery to the device.
        return "published_unconfirmed" if acknowledgements[0] == 0 else "failed"
    except (OSError, ValueError):
        return "uncertain" if sent else "failed"
    finally:
        client.disconnect()
