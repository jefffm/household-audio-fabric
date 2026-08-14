#!/bin/sh
set -eu
grep -q 'socat' /proc/1/cmdline
kill -0 1
# A blocked socat is healthy: it is applying downstream TCP backpressure to stdin.
