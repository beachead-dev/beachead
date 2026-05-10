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

    # --- Authorization header tests ---

    def test_valid_token_in_header_passes(self, app_with_auth):
        """Request with valid token in Authorization header should succeed."""
        response = app_with_auth.get(
            "/protected",
            headers={"Authorization": "Bearer test-secret-token"},
        )
        assert response.status_code == 200
        assert response.json() == {"message": "success"}

    def test_missing_auth_returns_401(self, app_with_auth):
        """Request without any token should get 401."""
        response = app_with_auth.get("/protected")
        assert response.status_code == 401
        assert response.json() == {"error": "Unauthorized"}

    def test_invalid_token_in_header_returns_401(self, app_with_auth):
        """Request with wrong token in header should get 401."""
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

    # --- Query parameter tests ---

    def test_valid_token_in_query_param_passes(self, app_with_auth):
        """Request with valid token as query parameter should succeed."""
        response = app_with_auth.get("/protected?token=test-secret-token")
        assert response.status_code == 200
        assert response.json() == {"message": "success"}

    def test_invalid_token_in_query_param_returns_401(self, app_with_auth):
        """Request with wrong token as query parameter should get 401."""
        response = app_with_auth.get("/protected?token=wrong-token")
        assert response.status_code == 401
        assert response.json() == {"error": "Unauthorized"}

    def test_empty_token_query_param_returns_401(self, app_with_auth):
        """Request with empty token query parameter should get 401."""
        response = app_with_auth.get("/protected?token=")
        assert response.status_code == 401

    # --- Priority tests ---

    def test_query_param_takes_priority_over_header(self, app_with_auth):
        """Query parameter token should be checked before header."""
        # Valid query param, invalid header — should pass (query param wins)
        response = app_with_auth.get(
            "/protected?token=test-secret-token",
            headers={"Authorization": "Bearer wrong-token"},
        )
        assert response.status_code == 200

    def test_invalid_query_param_not_rescued_by_valid_header(self, app_with_auth):
        """Invalid query param should fail even if header is valid."""
        # Invalid query param, valid header — should fail (query param checked first)
        response = app_with_auth.get(
            "/protected?token=wrong-token",
            headers={"Authorization": "Bearer test-secret-token"},
        )
        assert response.status_code == 401

    # --- Health endpoint ---

    def test_health_endpoint_exempt_from_auth(self, app_with_auth):
        """Health check endpoint should not require authentication."""
        response = app_with_auth.get("/health")
        assert response.status_code == 200
        assert response.json() == {"status": "healthy"}

    # --- Error response consistency ---

    def test_error_response_does_not_reveal_details(self, app_with_auth):
        """401 response should not reveal whether format or value was wrong."""
        # Wrong format
        r1 = app_with_auth.get(
            "/protected",
            headers={"Authorization": "NotBearer token"},
        )
        # Wrong value in header
        r2 = app_with_auth.get(
            "/protected",
            headers={"Authorization": "Bearer wrong"},
        )
        # Wrong value in query param
        r3 = app_with_auth.get("/protected?token=wrong")
        # No auth at all
        r4 = app_with_auth.get("/protected")
        # All should have identical error response
        assert r1.json() == r2.json() == r3.json() == r4.json() == {"error": "Unauthorized"}
