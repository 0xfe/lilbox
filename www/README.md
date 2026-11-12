# lilbox website

Public URL: **https://mo.town/lilbox/**. Storage destination:
**gs://muthanna.com/lilbox/** (bucket `muthanna.com`, object prefix `lilbox/`).

`index.html` is the entire page, with embedded CSS and JavaScript. There is no
framework, package manager, build step, analytics, or external JavaScript.
The page uses a native system sans-serif font. Code and the lilbox name use Cousine from Google
Fonts, with local monospace fallbacks. The icon is a local PNG with transparency.
Installation and navigation still work without JavaScript; JavaScript adds copy
buttons for the install command, configuration prompt, and run command. When clipboard access is unavailable, each button selects its text
for manual copying.

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
install card is followed by the product description and a combined Configure
and Run card for agent-assisted profile creation and launching Claude Code. Keep the configuration prompt copyable as plain text.
The “Why outer sandboxes?” section explains the outer sandbox and its role alongside other
isolation mechanisms. Avoid implying that the default profile is workspace-only.

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
