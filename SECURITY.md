# Security policy

## Supported versions

This project is pre-release. Security fixes are made only on the current `main` branch.

## Reporting a vulnerability

Do not open a public issue for an unpatched vulnerability. Use [GitHub private vulnerability reporting](https://github.com/jefffm/household-audio-fabric/security/advisories/new), which is enabled for this repository. Include the affected component, reproduction steps, impact, and any suggested mitigation. If GitHub cannot present that private form, disclose no sensitive details in a public issue; report only that the private channel is unavailable.

## Deployment expectations

No component should be directly exposed to the public Internet. Isolate the media LAN; restrict Snapcast HTTP/control ports; deliver authority bearer tokens through a secret store; rotate them after suspected disclosure; use read-only filesystems and the documented least capabilities; and verify the receiver host-firewall attestation. Snapcast has no built-in authentication or TLS in this design. Physical-device validation and container-runtime capability validation are required before production use.
