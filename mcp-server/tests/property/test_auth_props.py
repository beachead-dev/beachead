"""Property-based tests for token validation.

Tests that matching tokens are accepted and non-matching tokens are rejected.
"""

from hypothesis import given, settings
from hypothesis import strategies as st

from beachead_memory_mcp.auth import validate_token


# Strategy for generating token-like strings
token_strategy = st.text(
    alphabet=st.characters(
        whitelist_categories=("L", "N", "P", "S"),
        blacklist_characters="\x00\n\r",
    ),
    min_size=1,
    max_size=128,
)


class TestTokenValidationProperties:
    """Property-based tests for token validation."""

    @given(token=token_strategy)
    @settings(max_examples=100)
    def test_matching_token_always_accepted(self, token: str):
        """For any token string, comparing it to itself always returns True."""
        assert validate_token(token, token) is True

    @given(
        token_a=token_strategy,
        token_b=token_strategy,
    )
    @settings(max_examples=100)
    def test_different_tokens_rejected(self, token_a: str, token_b: str):
        """For any two different tokens, comparison returns False."""
        if token_a != token_b:
            assert validate_token(token_a, token_b) is False

    @given(
        base_token=st.text(min_size=2, max_size=64),
        suffix=st.text(min_size=1, max_size=10),
    )
    @settings(max_examples=100)
    def test_prefix_not_accepted(self, base_token: str, suffix: str):
        """A prefix of the expected token should never be accepted."""
        full_token = base_token + suffix
        if base_token != full_token:
            assert validate_token(base_token, full_token) is False

    @given(
        base_token=st.text(min_size=2, max_size=64),
        prefix=st.text(min_size=1, max_size=10),
    )
    @settings(max_examples=100)
    def test_extended_token_not_accepted(self, base_token: str, prefix: str):
        """An extended version of the expected token should not be accepted."""
        extended = prefix + base_token
        if extended != base_token:
            assert validate_token(extended, base_token) is False

    @given(expected=token_strategy)
    @settings(max_examples=100)
    def test_empty_provided_rejected_when_expected_nonempty(self, expected: str):
        """Empty provided token should not match non-empty expected."""
        if expected:
            assert validate_token("", expected) is False
