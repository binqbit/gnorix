# Test assets

[test-fingerprint.png](test-fingerprint.png) is a synthetic scanner-style image generated with ImageGen. No real biometric reference was supplied; it is not a capture from a physical sensor.

The original is preserved as a 1086 × 1448, 8-bit RGB PNG with unspecified physical resolution. Decode it to single-channel grayscale before supplying pixels to `Capture::new`. The report example performs this conversion.

Use it for repeatable [extraction diagnostics](../../../../docs/fingerprint.md), not recognition-accuracy or biometric-independence claims. External research inputs and their separate provenance are described in the [SOCOFing testing guide](../../../../docs/fingerprint-testing.md).
