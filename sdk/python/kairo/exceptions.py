"""Kairo Python SDK Exceptions."""


class KairoError(Exception):
    """Base exception for all Kairo SDK errors."""
    pass


class KairoConnectionError(KairoError):
    """Raised when the client fails to connect to the Kairo HTTP server."""
    pass


class KairoApiError(KairoError):
    """Raised when the Kairo HTTP server returns an HTTP 4xx or 5xx error."""

    def __init__(self, status_code: int, message: str, details: dict | None = None):
        super().__init__(f"HTTP {status_code}: {message}")
        self.status_code = status_code
        self.message = message
        self.details = details or {}
