# Runtime bundle staging

This directory is intentionally tracked so Tauri's `runtime/**/*` resource
glob has a match during development builds.

`scripts/bundle-runtime.sh` places the generated Colima, Lima, and Docker
binaries here for release builds. Those generated files remain git-ignored.
