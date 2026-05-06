"""Bearer token authentication for the MCP server.

SECURITY: Uses constant-time comparison (hmac.compare_digest) to prevent
timing attacks. Error responses do not reveal whether the token format
or value was incorrect.
"""

from __future__ import annotations

import hmac

from starlette.middleware.base import BaseHTTPMiddleware
from starlette.requests import Request
from starlette.responses import JSONResponse


class BearerTokenMiddleware(BaseHTTPMiddleware):
    """Middleware that validates Bearer token authentication.

    Rejects requests without a valid Authorization header with 401.
    The health check endpoint is exempt from authentication.
    """

    def __init__(self, app, expected_token: str) -> None:
        """Initialize with the expected bearer token.

        Args:
            app: The ASGI application to wrap.
            expected_token: The token value that clients must provide.
        """
        super().__init__(app)
        self._expected_token = expected_token

    async def dispatch(self, request: Request, call_next):
        """Validate the Authorization header on each request."""
        # Health check endpoint is exempt from auth
        if request.url.path == "/health":
            return await call_next(request)

        auth_header = request.headers.get("authorization", "")

        if not auth_header.startswith("Bearer "):
            return JSONResponse(
                status_code=401,
                content={"error": "Unauthorized"},
            )

        provided_token = auth_header[7:]  # Strip "Bearer " prefix

        # SECURITY: Constant-time comparison to prevent timing attacks
        if not hmac.compare_digest(
            provided_token.encode("utf-8"), self._expected_token.encode("utf-8")
        ):
            return JSONResponse(
                status_code=401,
                content={"error": "Unauthorized"},
            )

        return await call_next(request)


def validate_token(provided: str, expected: str) -> bool:
    """Validate a bearer token using constant-time comparison.

    Args:
        provided: The token provided by the client.
        expected: The expected valid token.

    Returns:
        True if tokens match, False otherwise.
    """
    # Encode to bytes for hmac.compare_digest to handle all characters
    return hmac.compare_digest(
        provided.encode("utf-8"), expected.encode("utf-8")
    )
