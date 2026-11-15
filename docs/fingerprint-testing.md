# Optional fingerprint testing with SOCOFing

For a first local test, use the bundled fingerprint in the [Lab](fingerprint-lab.md)
or generate the [PNG report](../tools/fingerprint-reports/README.md#run).
The [local testing guide](local-testing.md) covers setup, one-command launches
and automated checks. **SOCOFing is optional** for all of those workflows;
this page describes adding external research inputs afterward.

[SOCOFing on Kaggle](https://www.kaggle.com/datasets/ruizgara/socofing) provides external research inputs for ridge clarity, local normalization and feature extraction. Keep the dataset and converted captures outside tracked project files. Normal builds and tests must not require a Kaggle download.

For interactive demonstrations, the [Fingerprint Lab gallery](fingerprint-lab.md#choose-an-image) imports five pinned originals into an ignored local directory. Its visually curated examples are separate from the stratified comparison set described below. The manifest records source and decoded-pixel hashes; neither importing nor viewing resizes the analysis input.

## Provenance and use

Shehu, Ruiz-Garcia, Palade and James published SOCOFing in 2018: 6,000 originals from 600 adults, with ten fingers per subject. Images carry subject, gender-label, hand and finger labels. STRANGE-generated obliteration (`Obl`), central rotation (`CR`) and `Zcut` variants have Easy, Medium and Hard settings. The paper identifies Hamster plus HSDU03PTM and SecuGen SDU03PTM scanners. See the [dataset publication](https://arxiv.org/abs/1807.10609).

The version-2 metadata inspected for this project lists **Other (specified in description)**. The paper's [usage section](https://arxiv.org/pdf/1807.10609) limits use to noncommercial, not-for-profit research and requires both citations below in resulting publications, technical reports and manuals. Do not infer commercial or redistribution permission from public download access.

- Shehu et al., [Sokoto Coventry Fingerprint Dataset](https://arxiv.org/abs/1807.10609), 2018.
- Shehu et al., [Detection of Fingerprint Alterations Using Deep Convolutional Neural Networks](https://doi.org/10.1007/978-3-030-01418-6_6), ICANN 2018.

An altered image is a synthetic derivative of its named original. It is not a separately scanned impression of that finger. These data do not supply repeat-capture ground truth for registration, tolerance calibration or biometric-key recovery, nor annotated true ridge contours or minutiae. Keep all derivatives of a subject together if making development/evaluation splits.

## Verified version-2 files

A complete download and BMP inspection on 2026-11-12 found the following in `SOCOFing/`:

| Directory | Images | 96 × 103 | 241 × 298 |
| --- | ---: | ---: | ---: |
| `Real` | 6,000 | 5,956 | 44 |
| `Altered/Altered-Easy` | 17,931 | 17,803 | 128 |
| `Altered/Altered-Medium` | 17,067 | 16,938 | 129 |
| `Altered/Altered-Hard` | 14,272 | 14,140 | 132 |
| Total | 55,270 | 54,837 | 433 |

The paper reports 17,934 Easy images and 55,273 overall; the downloaded version has three fewer. It also contains a second `socofing/SOCOFing/` tree, verified byte-identical for every relative path. Count only one tree. One copy occupies 810,626,156 bytes.

The paper describes 500 dpi, whereas BMP headers declare approximately 96 dpi for standard originals and alterations, and 120 dpi for larger originals. Neither header values nor rescaling establishes physical calibration. Gnorix measures source pixels and estimates its own ridge unit; do not inject 500 dpi or resize to match an assumed sensor resolution.

## Retrieve a pinned copy

Use the official [KaggleHub client](https://github.com/Kaggle/kagglehub/tree/v1.0.3) with an explicit version and a directory outside the checkout or in an ignored location:

```sh
export SOCOFING_DOWNLOAD=/absolute/path/to/local-data/socofing
uv run --with kagglehub==1.0.3 python - <<'PY'
import os
import kagglehub

print(kagglehub.dataset_download(
    "ruizgara/socofing/versions/2",
    output_dir=os.environ["SOCOFING_DOWNLOAD"],
))
PY
```

Use `$SOCOFING_DOWNLOAD/SOCOFing` as the source tree. The recorded download required no authentication; follow Kaggle's access requirements if they change. Record dataset version, relative input paths, source SHA-256 values, conversion details and extractor revision with each evaluation.

## Convert without inventing scanner evidence

The report command accepts opaque PNG. All 6,000 original BMPs have equal red, green and blue channels. However, all 5,956 standard originals decode as RGBA: their only observed area is the fully opaque rectangle `(left=0, top=0, right=92, bottom=99)`. The final four columns and four rows are transparent black padding. Dropping alpha would turn that padding into false dark evidence.

Crop these originals to the opaque rectangle and preserve its grayscale bytes exactly. The output is 92 × 99 with unchanged pixel scale and zero coordinate offset. The 44 larger originals are opaque RGB and remain 241 × 298. Altered BMPs decode as grayscale without alpha; for paired comparison, apply the corresponding original's crop to them too. Record the crop box and original path. If a different input has partial alpha or interior missing pixels, use the library's `Capture` validity mask instead of this crop procedure.

For one original, from the repository root:

```sh
uv run --project tools/fingerprint-reports --locked python - \
  "$SOCOFING_DOWNLOAD/SOCOFing/Real/13__F_Right_little_finger.BMP" \
  "$SOCOFING_DOWNLOAD/13__F_Right_little_finger.png" <<'PY'
import sys
from PIL import Image

with Image.open(sys.argv[1]) as source:
    box = (0, 0, source.width, source.height)
    if "A" in source.getbands():
        alpha = source.getchannel("A")
        box = alpha.getbbox()
        if box is None or alpha.crop(box).getextrema() != (255, 255):
            raise ValueError("Input needs a validity mask, not alpha removal")

    red, green, blue = source.convert("RGB").crop(box).split()
    if red.tobytes() != green.tobytes() or red.tobytes() != blue.tobytes():
        raise ValueError("Unexpected colored input")
    red.save(sys.argv[2], format="PNG")
    with Image.open(sys.argv[2]) as result:
        assert result.mode == "L" and result.size == red.size
        assert result.tobytes() == red.tobytes()
    print("Source size:", source.size, "crop:", box, "PNG size:", red.size)
PY

uv run --project tools/fingerprint-reports --locked gnorix-report \
  --image "$SOCOFING_DOWNLOAD/13__F_Right_little_finger.png"
```

The final command writes the single `reports/fingerprint/` bundle; see the [report refresh boundary](../tools/fingerprint-reports/README.md#refresh-and-recovery). Insufficient supported ridge measurements produce an explicit failure. Retain failures in evaluation records.

## Reproducible comparison set

The recorded development set contains 22 originals and 36 matched alterations. Choose the smallest SHA-256 of the UTF-8 relative path `Real/<filename>` in each gender-label × hand × finger group, then add the smallest per gender-label among the larger originals. This gives 20 stratified originals plus two larger-image controls. Selection precedes processing and does not depend on successful extraction.

For alteration comparisons, choose the smallest selected standard original per gender-label and the two larger controls. Version 2 yields `13__F_Right_little_finger`, `333__M_Left_thumb_finger`, `560__F_Left_little_finger` and `590__M_Left_ring_finger`; each has all nine alteration combinations. These 58 inputs exercise distinct source conditions. They are a development sample, not a population-representative accuracy benchmark.

Compare original source intensities, local dark/light references, supported coverage, measured widths, clarity diagnostics and inferred geometry. Preserve input failures and unsupported areas rather than replacing them with a successful example. Synthetic alteration severity is not a calibrated scan-quality score, and smoother output alone does not establish more accurate biometric features.

For paired clarity comparisons, use the intersection of both clarity-support masks and the original's confirmed-pattern region. Keep that original region fixed: an alteration must not improve its measured result merely by changing which pixels count as pattern. Record the common pixel count alongside each mean. Prefer native floating-point maps/segment values; exported PNG scores are quantized to `1/255`, so paired means have a worst-case rounding uncertainty of `±1/255`.

Higher clarity after a synthetic alteration can reflect sharpened or regularized local structure; it does not establish authenticity. Test repeat captures and unrelated subjects separately for key recovery and false matches. The current descriptor's [known cross-subject limitation](fingerprint-signing.md) is not resolved by successful extraction or a valid signature proof.
