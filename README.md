# dave-wang-6c2c-daily-video

Rust service for generating the daily animal video pipeline: select the next animal, generate a funny clip, extract a representative frame, create a Meshy GLB, render a Blender print-reveal, concatenate the final MP4 with ffmpeg, upload artifacts to private object storage, and expose a public video feed.

## Runtime API

- `GET /health` - service health check.
- `GET /videos?limit=10&offset=0` - newest-first published video feed.
- `GET /videos/latest` - latest published video.
- `POST /admin/runs` - manually trigger a run. Requires `Authorization: Bearer <ADMIN_API_KEY>`.
- `POST /admin/runs/{id}/retry` - retry a failed run from the last good step. Requires the same admin API key.

## Host Requirements

This repository is prepared for a bare self-hosted deployment. It intentionally does not include a Dockerfile or CI/CD workflow.

Install these on the host:

- Rust and Cargo for building the service.
- PostgreSQL 16 or a reachable PostgreSQL-compatible endpoint via `DATABASE_URL`.
- `ffmpeg` in `PATH`, with H.264/AAC support for frame extraction and final MP4 assembly.
- `blender` in `PATH`, able to run headless with `--background` for GLB turntable rendering.
- CA certificates and outbound network access to Gemini/Veo, Meshy, PostgreSQL, and Tigris object storage.
- A writable pipeline workspace directory, such as `/var/lib/dave-wang-6c2c-daily-video/work`.

On Debian or Ubuntu hosts, the host binaries are typically installed with:

```bash
sudo apt-get update
sudo apt-get install -y ffmpeg blender ca-certificates
ffmpeg -version
blender --background --version
```

Some Linux servers need additional GPU, EGL, OSMesa, or Xvfb packages for Blender headless rendering. Validate `blender --background --version` under the same user that will run the service.

## Environment

Copy `.env.example` to `.env` or load the variables through your process manager. Do not commit real credentials.

Required configuration:

- `DATABASE_URL` must point at PostgreSQL. SQLite, JSON-file persistence, and local-only persistent state are not supported.
- `OBJECT_STORAGE_ACCESS_KEY_ID`, `OBJECT_STORAGE_SECRET_ACCESS_KEY`, `OBJECT_STORAGE_BUCKET`, `OBJECT_STORAGE_PREFIX`, `OBJECT_STORAGE_ENDPOINT`, `OBJECT_STORAGE_REGION`, and `OBJECT_STORAGE_FORCE_PATH_STYLE` are required exactly as named.
- `OBJECT_STORAGE_PREFIX` must end with `/`. Every generated artifact key is stored under that prefix.
- The object storage bucket is private. The service stores object keys and returns signed read URLs from feed endpoints.
- `GEMINI_API_KEY` and `MESHY_API_KEY` are required for provider calls.
- `ADMIN_API_KEY` should be a long random secret sent as `Authorization: Bearer <ADMIN_API_KEY>`.
- `SCHEDULE_TIME` is `HH:MM`, interpreted in `SCHEDULE_TIMEZONE`.

Optional configuration:

- `HOST` defaults to `0.0.0.0`.
- `PORT` defaults to `8080`.
- `PIPELINE_WORKSPACE_DIR` defaults to `/tmp/dave-wang-6c2c-daily-video`.
- `RUST_LOG` controls structured application logging.

## Bare Host Run

Create a local workspace for temporary media files:

```bash
sudo mkdir -p /var/lib/dave-wang-6c2c-daily-video/work
sudo chown "$USER":"$USER" /var/lib/dave-wang-6c2c-daily-video/work
```

Build and start the service:

```bash
cargo build --release
set -a
. ./.env
set +a
./target/release/dave-wang-6c2c-daily-video
```

The service binds to `HOST:PORT` and runs embedded database migrations at startup. Put a reverse proxy in front of it if you need TLS or a public hostname.

## Operations

Health check:

```bash
curl http://127.0.0.1:8080/health
```

Read the public feed:

```bash
curl 'http://127.0.0.1:8080/videos?limit=10&offset=0'
curl http://127.0.0.1:8080/videos/latest
```

Trigger a run for a specific date and animal:

```bash
curl -X POST http://127.0.0.1:8080/admin/runs \
  -H "Authorization: Bearer $ADMIN_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"date":"2026-07-01","animal":"cat"}'
```

Retry a failed run:

```bash
curl -X POST "http://127.0.0.1:8080/admin/runs/<run-id>/retry" \
  -H "Authorization: Bearer $ADMIN_API_KEY"
```

Generated intermediate artifacts and final MP4 files are uploaded to object storage. The local pipeline workspace is temporary working space, not the source of truth.
