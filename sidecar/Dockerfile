# syntax=docker/dockerfile:1.7

FROM ghcr.io/astral-sh/uv:python3.12-bookworm-slim AS builder

WORKDIR /app

ENV UV_LINK_MODE=copy \
    UV_COMPILE_BYTECODE=1 \
    UV_PROJECT_ENVIRONMENT=/app/.venv

COPY pyproject.toml uv.lock README.md ./
RUN --mount=type=cache,target=/root/.cache/uv \
    uv sync --frozen --no-dev --no-install-project

COPY src ./src
RUN --mount=type=cache,target=/root/.cache/uv \
    uv sync --frozen --no-dev


FROM python:3.12-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends libsndfile1 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --shell /bin/bash --uid 10001 sidecar

WORKDIR /app

ENV PATH=/app/.venv/bin:$PATH \
    PYTHONUNBUFFERED=1

COPY --from=builder --chown=sidecar:sidecar /app/.venv /app/.venv
COPY --chown=sidecar:sidecar src /app/src
COPY --chown=sidecar:sidecar personas /app/personas

USER sidecar

EXPOSE 8080

CMD ["python", "-m", "stackchan_sidecar"]
