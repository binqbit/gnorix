# Demo fingerprint fixtures

The NEAR demo uses two independently generated synthetic grayscale fingerprints:

| Wallet | Asset | Pattern | Dimensions | PNG size |
| --- | --- | --- | --- | --- |
| 1 | [wallet-1-fingerprint.png](wallet-1-fingerprint.png) | Broad right-side loop | 241 × 298 | 62,233 bytes |
| 2 | [wallet-2-fingerprint.png](wallet-2-fingerprint.png) | Off-center whorl | 241 × 298 | 59,509 bytes |

Both were generated with the built-in ImageGen tool on 2026-11-14. The appearance
was specified after inspecting the locally imported SOCOFing gallery entries
226/right-middle and 270/right-index: dark contact ridges, light background and
compact scan dimensions. Those gallery images were **not** supplied to the
generator; both patterns were generated independently from the prompts below.
No SOCOFing scan is bundled in these files.

Each native output was 1128 × 1394 pixels. The final asset was converted to
8-bit grayscale and resized to 241 × 298 using Lanczos resampling, then saved as
an optimized PNG. This matches the gallery's dimensions without retaining HD
image data. No hand-authored ridge edits or synthetic feature overlays were
added after generation. The core test fingerprint is unchanged.

The two files are software-test fixtures, not scans of identified people.
Physical resolution and sensor calibration are unspecified. Distinct generated
patterns do not establish biometric independence, recognition accuracy or
cryptographic security.

SHA-256 of the files consumed by the demo:

- Wallet 1: `aea2bb054f6b83a6fa313605e06d41a6c2933d8ca02a7b9c7f6a88f20fe240bc`
- Wallet 2: `07978c10378ff65e4e410d484db6d8e71dd474b6ca96becce383111f4438a6e5`

SHA-256 of the native generated outputs before conversion:

- Wallet 1: `80c059a6499ce023ef02579fe45e40240d883fff9d78ff1d7f5344f7524534bd`
- Wallet 2: `1be3f34c260175c766dd0421fd12e631c7bc6c8eebae7159e9b41c063b460b8f`

<details>
<summary>Exact generation prompts</summary>

### Wallet 1

```text
Use case: scientific-educational. Asset: one synthetic fingerprint scan fixture for a local software demonstration. Create a completely fictional fingerprint pattern, not copied from any real person's print. A raw grayscale contact fingerprint raster in a portrait rectangular scanner frame with aspect ratio 241:298. Black and dark charcoal fingerprint ridges on a white to very light gray background, like a small laboratory fingerprint dataset scan. Pattern: a broad upright loop bending toward the upper right, with a distinctive off-center loop core in the right half and a coherent fan of sweeping long ridges underneath. The oval contact area nearly fills the frame, leaving a modest light margin around the upper left; the lower edge can naturally extend beyond the frame. At the final small display resolution there should be about24–30 clearly separated ridges across the width, with typical dark ridge width4–6pixels and ridge-plus-valley period8–12pixels in a241×298image. Keep ridges mostly long and continuous, with a few natural, clear, spatially distributed true ridge endings and bifurcations; do not make a perfectly regular stripe pattern. Subtle uneven contact density and slightly ragged ridge edges are desirable, but avoid strong blur, excessive fragments, disconnected islands, solid black blobs or oversharpened outlines. White valleys must remain readable. This is the fingerprint pattern only: no visible finger, skin silhouette with a stem, hand, fingernail, paper, scanner device, borders, grids, colored overlays, markers, circles, text, labels or watermark. Produce one portrait grayscale image, not a collage. The synthetic input will later be reduced to241×298pixels; prioritize readable ridge structure over photographic HD detail.
```

### Wallet 2

```text
Use case: scientific-educational. Asset: one independent synthetic fingerprint scanner raster for a local software demonstration. Create a completely fictional ridge pattern, not any real person's fingerprint. Portrait rectangular raw scan, aspect ratio241:298. Dark charcoal and nearly black contact ridges on a white/light-gray background, resembling a compact laboratory fingerprint dataset capture. Pattern: an off-center rounded whorl slightly LEFT of the middle with a small coherent spiral core, surrounded by long sweeping oval and arch ridges, with a distinct lower-left delta. The contact pattern fills most of the frame, with narrow pale margins at the top and sides, and ridges naturally running off the lower edge. Give it a different overall ridge geometry from an ordinary right-side loop. Keep the structure readable when reduced to241×298pixels: typical ridge-plus-valley spacing8–12pixels, dark ridge width4–6pixels, around24–30 ridges across the broadest dimension. Add a small number of natural distributed ridge endings and bifurcations, without conspicuous straight line cuts or geometric white wedges. Mild contact irregularity and slightly rough ridge edges, softly varied density; predominantly continuous separated ridges with clear light valleys. The image must look like a grayscale digital scan with moderate fine grain, not computer vector art, an icon, a color illustration or a realistic photograph of a finger. Avoid excessive speckles, tiny pores, broken fragments, overlapping solid black masses, strong blur, artificial symmetry, perfectly even strokes, paper texture, ink splatter, visible finger body, hand, fingernail, frame, border, grid, markings, labels, circles, overlay, text and watermark. One portrait image only, no collage. The output is a synthetic software-test input; prioritize practical low-resolution ridge geometry, not HD detail.
```

The prompts describe the requested appearance; their ridge dimensions are
generation targets rather than measured or certified biometric properties.

</details>
