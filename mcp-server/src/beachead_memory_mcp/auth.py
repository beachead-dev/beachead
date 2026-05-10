"""Bearer token authentication for the MCP server.

Supports two authentication methods:
1. Query parameter: ?token=<value> (preferred — works with all MCP clients)
2. Authorization header: Bearer <value> (fallback for clients that support headers)

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
    """Middleware that validates bearer token authentication.

    Accepts tokens via:
    - Query parameter: ?token=<value> (checked first)
    - Authorization header: Bearer <value> (fallback)

    Rejects requests without a valid token with 401.
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
        """Validate the token on each request."""
        # Health check endpoint is exempt from auth
        if request.url.path == "/health":
            return await call_next(request)

        provided_token = self._extract_token(request)

        if provided_token is None:
            return JSONResponse(
                status_code=401,
                content={"error": "Unauthorized"},
            )

        # SECURITY: Constant-time comparison to prevent timing attacks
        if not hmac.compare_digest(
            provided_token.encode("utf-8"), self._expected_token.encode("utf-8")
        ):
            return JSONResponse(
                status_code=401,
                content={"error": "Unauthorized"},
            )

        return await call_next(request)

    def _extract_token(self, request: Request) -> str | None:
        """Extract token from query parameter or Authorization header.

        Checks query parameter first (preferred for MCP client compatibility),
        then falls back to Authorization header.

        Returns:
            The token string, or None if no token was provided.
        """
        # Method 1: Query parameter ?token=<value>
        token_param = request.query_params.get("token")
        if token_param:
            return token_param

        # Method 2: Authorization header (Bearer <value>)
        auth_header = request.headers.get("authorization", "")
        if auth_header.startswith("Bearer "):
            return auth_header[7:]  # Strip "Bearer " prefix

        return None


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
