# lilbox website

Public URL: **https://mo.town/lilbox/**. Storage destination:
**gs://muthanna.com/lilbox/** (bucket `muthanna.com`, object prefix `lilbox/`).

`index.html` is the entire page, with embedded CSS and JavaScript. There is no
framework, package manager, build step, analytics, or external JavaScript.
Cousine is loaded from Google Fonts, matching mo.town, with local monospace
fallbacks. The icon is a local PNG with transparency. Installation and navigation
still work without JavaScript; JavaScript only adds the copy button. When clipboard
access is unavailable, the button selects the command for manual copying.

## Serve locally

From the repository root, with Python 3 installed:

```sh
just serve
# Or, without Just:
python3 -m http.server 8000 --bind 127.0.0.1 --directory www
```

Open **http://127.0.0.1:8000/**. Stop the server with Ctrl-C. To choose another port,
replace `8000` in the Python command. Relative asset paths work both locally and
under `/lilbox/`; the install command intentionally points to the public installer.

## Check and upload

```sh
just test-www
just upload
```

Uploading needs Python 3, Bash, and an authenticated Google Cloud CLI with write
access to the bucket. `scripts/upload.sh` checks the page and asset fingerprints,
freezes a local snapshot, and uploads:

1. `www/assets/` to `gs://muthanna.com/lilbox/assets/`, with public-read access and
   `Cache-Control: public,max-age=86400,immutable`.
2. `www/index.html` to `gs://muthanna.com/lilbox/index.html`, with public-read access,
   `Content-Type: text/html`, and `Cache-Control: public,max-age=300,must-revalidate`.

The page is uploaded only after the assets succeed, then its bytes are checked
with a GCS readback. Caches may take five minutes to show an updated page.
Old image objects are retained for cached pages. Avoid simultaneous uploads.
The uploader does not delete bucket objects or upload this README. It never
changes `lilbox/install.sh`; use `just release-installer` for that. CI checks and
deployment tests are offline and never publish.

## Editing

Edit text, styles, and the small copy handler directly in `index.html`. Keep the
`lilbox` spelling and the install URL `https://mo.town/lilbox/install.sh`. The
isolated-agent example requires the profile to be configured for the agent's
paths, state, credentials, and network needs; the shipped isolated profile is
closed and offline. Avoid implying that the default profile is workspace-only.

Images use the first 12 hex characters of their SHA-256 digest in the filename.
When replacing an image, give it a new fingerprinted name and update its references
in `index.html` (hero, favicon, and Open Graph image). `just test-www` catches stale
fingerprints and missing assets. For example:

```sh
shasum -a 256 www/assets/*.png
```

The generated icon is `assets/lilbox-84d2985fa696.png`. It was created using the
built-in image generator and copied into the repository with its original alpha
channel; no API key or image-generation dependency is needed to serve the site.

Generation prompt:

> Use case: logo-brand. Asset type: small website icon for lilbox, a minimal command-line OS sandbox. Create one beautiful, simple isometric open box holding a smaller solid cube, suggesting an outer layer of protection. Crisp softly beveled edges, understated tactile matte finish, warm coral-red exterior with off-white interior and charcoal details. Restrained friendly editorial style suitable for a very minimal white monospace developer website. Centered square composition, generously padded, legible at 128px, clean silhouette. Truly transparent background, subtle contact shading only, no floor, no border, no words, no letters, no lock or shield, no extra objects, no watermark.
