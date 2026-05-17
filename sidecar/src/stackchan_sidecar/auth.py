import secrets
from collections.abc import Callable

from fastapi import Header, HTTPException, Request, status

_BEARER_PREFIX = "Bearer "


def make_verifier(expected_token: str) -> Callable[[Request, str], None]:
    def verify_bearer(
        request: Request,
        authorization: str = Header(default=""),
    ) -> None:
        if not authorization or not authorization.startswith(_BEARER_PREFIX):
            raise HTTPException(
                status_code=status.HTTP_401_UNAUTHORIZED,
                detail="missing or malformed Authorization header",
                headers={"WWW-Authenticate": "Bearer"},
            )
        presented = authorization[len(_BEARER_PREFIX) :].strip()
        if not secrets.compare_digest(presented, expected_token):
            raise HTTPException(
                status_code=status.HTTP_401_UNAUTHORIZED,
                detail="invalid bearer token",
                headers={"WWW-Authenticate": "Bearer"},
            )
        request.state.authenticated = True

    return verify_bearer
