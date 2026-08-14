# Compliance summary

The image ships verbatim upstream notices at:

- `/usr/share/doc/shairport-sync/COPYING` and `/usr/share/doc/shairport-sync/LICENSES`
- `/usr/share/doc/nqptp/COPYING` and `/usr/share/doc/nqptp/LICENSE`
- Debian package copyright files under `/usr/share/doc/*/copyright`

Shairport Sync declares MIT for its own code but incorporates separately licensed components recorded in its `LICENSES` notice file. NQPTP is GPL-2.0-only. Debian FFmpeg libraries may include GPL-2.0-or-later code/configuration; the aggregate OCI license annotation is intentionally omitted because no supportable whole-image SPDX expression has been established; consult each component notice and the generated package inventory rather than treating an aggregate expression as a complete image conclusion. Anyone distributing the binary image must inspect the generated SPDX package inventory, retain notices, and provide corresponding source/source offers as each GPL/LGPL component requires.
