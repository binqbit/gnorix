# Demo fingerprint fixtures

`wallet-2-fingerprint.png` is a synthetic scanner-style image generated with the
built-in ImageGen tool on 2026-11-14. The existing
[core test image](../../../crates/core/tests/assets/README.md) was supplied as the
reference for its gray optical-scanner appearance, ridge scale, soft edges and
contact texture. The generation varies the ridge pattern and loop position.
No real biometric reference was supplied.

The preserved output is a 1086 × 1448, 8-bit RGB PNG. The demo converts it to
grayscale before analysis; physical resolution is unspecified. Wallet 1 keeps
the original core fixture unchanged. These are related synthetic test inputs,
not evidence of distinct real identities, calibrated scanner output, or biometric
security. Neither image comes from SOCOFing; that dataset is not bundled.

SHA-256: `a509130b8a486c5aaff4042d0ad1ef401dc104cac3e083f259be4f1c60d0a0d5`.

<details>
<summary>Exact generation prompt</summary>

```text
Use case: scientific-educational. Asset: a second synthetic fingerprint raster for testing the same image-analysis pipeline. Input image 1 is the exact scanner-style reference: match its photographic gray optical-sensor appearance, portrait proportions, ridge scale, moderate contrast, soft ridge edges, illumination gradients and subtle granular skin/contact texture. Create a modest variation of the ridge pattern: a naturally curved loop in a slightly different position, with different spatially distributed true ridge endings and bifurcations. Keep most ridges long and continuous and the valleys readable, like the reference. The entire image must resemble that same softly focused grayscale digital fingerprint scan. This is a raw raster image filling the frame, not a finger silhouette or an ink print on paper. Preserve middle-gray valleys and charcoal-gray ridges rather than white and black. Texture should be subtle and comparable to the reference, without inventing excessive tiny islands, pores or fragmented ridges. Do not clean it into a vector drawing or clipart: no perfectly uniform line thickness, pure white background, bold black outlines, stylized graphic marks or sharp smooth computer-drawn curves. No color, labels, overlays, circles, text, watermark, fingernail or hand. Output one portrait image with approximately the same framing and resolution as the reference. The reference is synthetic; this output is also a synthetic software-test fixture, not a real person's biometric scan.
```

The prompt describes the intended appearance; it is not a measurement or
certification of the generated fingerprint.

</details>
