"""Host tooling for the stackchan-kai wake-word training loop.

The firmware exposes `behavior.audio_debug_udp_target` which forwards
every 20 ms ES7210 mic frame as a raw little-endian s16 UDP datagram
at 16 kHz mono. This package captures that stream into WAV files
shaped for microWakeWord training — the same input format the
``microwakeword`` Python package expects when building a new
``.tflite`` model.
"""

__all__ = ["__version__"]
__version__ = "0.1.0"
