# Media fixtures

`media.pdf`, `media.wav`, and `media.webm` are generated synthetic examples (PDF pages, a short sine wave, and a recorded canvas).
`encrypted-attachment.png` is a 320 × 200 PNG exported in Chromium from the original canvas drawing in the `EncryptedWithAttachments` story.
`sample.eml`, `sample.zip`, and the Office/OpenDocument fixtures are synthetic minimal parser samples without external dependencies.
The Office samples contain only the XML parts needed by the text extractor; they test preview extraction rather than full Office rendering.

`sample.msg` is `test/A memo.msg` from https://github.com/HiraokaHyperTools/msgreader (Apache-2.0); see `LICENSE.msgreader.txt`.

`cjk.pdf` is a synthetic Type0/CIDFont sample using `90ms-RKSJ-H` for あい. `jpx.pdf` is `test/pdfs/jpx_smaskindata.pdf` from https://github.com/mozilla/pdf.js (Apache-2.0; see `LICENSE.pdfjs.txt`). They exercise bundled CMap and JPEG 2000 decoder resources.
