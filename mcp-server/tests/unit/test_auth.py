"""Unit tests for bearer token authentication."""

import pytest

from beachead_memory_mcp.auth import validate_token


class TestValidateToken:
    """Tests for the validate_token function."""

    def test_matching_tokens_accepted(self):
        """Matching tokens should return True."""
        assert validate_token("secret-token-123", "secret-token-123") is True

    def test_non_matching_tokens_rejected(self):
        """Non-matching tokens should return False."""
        assert validate_token("wrong-token", "correct-token") is False

    def test_empty_provided_token_rejected(self):
        """Empty provided token should not match non-empty expected."""
        assert validate_token("", "expected-token") is False

    def test_empty_expected_token_rejected(self):
        """Non-empty provided should not match empty expected."""
        assert validate_token("some-token", "") is False

    def test_both_empty_accepted(self):
        """Both empty tokens should match (edge case)."""
        assert validate_token("", "") is True

    def test_unicode_tokens(self):
        """Unicode tokens should be compared correctly."""
        token = "tökën-with-üñîcödé"
        assert validate_token(token, token) is True
        assert validate_token(token, "different") is False

    def test_long_tokens(self):
        """Long tokens should be compared correctly."""
        token = "a" * 1000
        assert validate_token(token, token) is True
        assert validate_token(token, "a" * 999 + "b") is False

    def test_similar_tokens_rejected(self):
        """Tokens differing by one character should be rejected."""
        assert validate_token("token-abc", "token-abd") is False

    def test_prefix_tokens_rejected(self):
        """A prefix of the expected token should be rejected."""
        assert validate_token("token", "token-extended") is False

    def test_suffix_tokens_rejected(self):
        """A suffix of the expected token should be rejected."""
        assert validate_token("token-extended", "token") is False


class TestBearerTokenMiddleware:
    """Tests for the BearerTokenMiddleware ASGI middleware."""

    @pytest.fixture
    def app_with_auth(self):
        """Create a test Starlette app with auth middleware."""
        from starlette.applications import Starlette
        from starlette.responses import JSONResponse
        from starlette.routing import Route
        from starlette.testclient import TestClient

        from beachead_memory_mcp.auth import BearerTokenMiddleware

        async def protected_endpoint(request):
            return JSONResponse({"message": "success"})

        async def health_endpoint(request):
            return JSONResponse({"status": "healthy"})

        app = Starlette(
            routes=[
                Route("/protected", protected_endpoint),
                Route("/health", health_endpoint),
            ],
        )
        app.add_middleware(BearerTokenMiddleware, expected_token="test-secret-token")
        return TestClient(app)

    def test_valid_token_passes(self, app_with_auth):
        """Request with valid token should succeed."""
        response = app_with_auth.get(
            "/protected",
            headers={"Authorization": "Bearer test-secret-token"},
        )
        assert response.status_code == 200
        assert response.json() == {"message": "success"}

    def test_missing_auth_header_returns_401(self, app_with_auth):
        """Request without Authorization header should get 401."""
        response = app_with_auth.get("/protected")
        assert response.status_code == 401
        assert response.json() == {"error": "Unauthorized"}

    def test_invalid_token_returns_401(self, app_with_auth):
        """Request with wrong token should get 401."""
        response = app_with_auth.get(
            "/protected",
            headers={"Authorization": "Bearer wrong-token"},
        )
        assert response.status_code == 401
        assert response.json() == {"error": "Unauthorized"}

    def test_non_bearer_scheme_returns_401(self, app_with_auth):
        """Request with non-Bearer scheme should get 401."""
        response = app_with_auth.get(
            "/protected",
            headers={"Authorization": "Basic dXNlcjpwYXNz"},
        )
        assert response.status_code == 401

    def test_health_endpoint_exempt_from_auth(self, app_with_auth):
        """Health check endpoint should not require authentication."""
        response = app_with_auth.get("/health")
        assert response.status_code == 200
        assert response.json() == {"status": "healthy"}

    def test_error_response_does_not_reveal_details(self, app_with_auth):
        """401 response should not reveal whether format or value was wrong."""
        # Wrong format
        r1 = app_with_auth.get(
            "/protected",
            headers={"Authorization": "NotBearer token"},
        )
        # Wrong value
        r2 = app_with_auth.get(
            "/protected",
            headers={"Authorization": "Bearer wrong"},
        )
        # Both should have identical error response
        assert r1.json() == r2.json() == {"error": "Unauthorized"}
