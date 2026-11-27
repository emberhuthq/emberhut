# site

The public website, served at [emberhut.com](https://emberhut.com) and `www.emberhut.com`
by Cloudflare Pages: the landing page, the docs placeholder, the privacy and terms pages and
the 404 page.

It's plain HTML and one stylesheet, with no build step, no JavaScript, no web fonts and no
third-party requests, so every page is a single small download that works without a bundler.

```
index.html          the landing page
docs/ privacy/ terms/  one index.html each
404.html            served by Pages for any unknown path
styles.css          every page's styles, light and dark from prefers-color-scheme
_headers            response headers: CSP (no scripts), HSTS, caching
favicon.svg, apple-touch-icon.png, og.png   icons and the link preview image
robots.txt, sitemap.xml
```

The logo is a placeholder mark (a hut outline with a glowing coal), inlined in each page as
SVG; swap the `svg.mark` blocks and the icon files when the final logo lands.

Motion is CSS only: the coal's slow glow, a few sparks in the hero, and sections rising into
place as they scroll into view where the browser supports scroll-driven animation. All of it is off
under `prefers-reduced-motion`.

Pages and the zone's cache keep `styles.css` for a day, so each page links it as
`/styles.css?v=N`: bump `N` in every page when the stylesheet changes.

## Preview

```sh
python3 -m http.server 8788 --directory site
```

## Deploy

```sh
wrangler pages deploy site --project-name emberhut --branch main
```

with `CLOUDFLARE_API_TOKEN` (Pages edit) and `CLOUDFLARE_ACCOUNT_ID` set. The Pages project
`emberhut` has `emberhut.com` and `www.emberhut.com` as custom domains.
