# stackchan-sidecar

Python voice agent sidecar that the stackchan-kai firmware POSTs raw PCM to.
It runs STT (faster-whisper), routes the transcript through an LLM (Anthropic
Claude by default), and returns a short reply for the avatar's toast band plus
an emotion enum for the face.

## Wire contract

The firmware POSTs `audio/L16;rate=16000;channels=1` little-endian s16 PCM
(up to 30 s) to a configurable path; this sidecar serves it on
`POST /v1/listen`. Each request carries:

- `Authorization: Bearer <token>` — when the firmware operator has set
  `agent_sidecar_token`. The sidecar requires a token (`SIDECAR_BEARER_TOKEN`)
  and rejects unauthenticated requests with `401`.
- `X-Session-Id: <uuidv4>` — always present, persisted per device. The sidecar
  reads it for logging today and will use it for per-session memory later.

Response is JSON:

```json
{"text": "hi friend!", "emotion": "happy"}
```

`text` is shown on a 32-character toast band, so the sidecar clamps it
defensively. `emotion` is one of `neutral`, `happy`, `sleepy`, `surprised`,
`sad`, `angry`. Unknown values fall back to `neutral`.

The firmware-side schema and request layout live in `docs/sidecar.md` in the
stackchan-kai firmware repo.

## Setup

You need [`uv`](https://docs.astral.sh/uv/) and Python 3.12 or newer.

```bash
cd sidecar/
uv sync
cp .env.example .env
# edit .env: set SIDECAR_BEARER_TOKEN and ANTHROPIC_API_KEY
```

## Run

```bash
uv run stackchan-sidecar
```

or equivalently:

```bash
uv run python -m stackchan_sidecar
```

The first run downloads the `base.en` faster-whisper weights into the default
HuggingFace cache (`~/.cache/huggingface`).

## Smoke test

With the sidecar running on `localhost:8080`:

```bash
curl -s http://localhost:8080/healthz | jq

# Synthesize one second of silence and POST it. Replace TOKEN with whatever
# you set in .env.
TOKEN="$(grep ^SIDECAR_BEARER_TOKEN .env | cut -d= -f2-)"
head -c 32000 /dev/zero | curl -s \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: audio/L16;rate=16000;channels=1" \
  -H "X-Session-Id: 00000000-0000-4000-8000-000000000000" \
  --data-binary @- \
  http://localhost:8080/v1/listen | jq
```

## Personas

Each `personas/<name>.md` is the full system prompt for that persona. Pick one
via `config.toml`:

```toml
persona = "stack-chan"
```

A persona file may carry an optional YAML frontmatter block (between `---`
lines at the start of the file) — the sidecar strips it before sending the
prompt to the model, so you can keep metadata next to the prompt without
polluting it.

## Quality gates

```bash
uv run ruff format --check .
uv run ruff check .
uv run mypy .
uv run pytest
```

CI runs all four on any PR touching `sidecar/**`.
