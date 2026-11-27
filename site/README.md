# site

The public website, served at [emberhut.com](https://emberhut.com) by Cloudflare Pages: the home page, the
docs placeholder, the privacy and terms pages, the pairing page and the 404 page.

It's hand-written HTML and CSS with a few small ES modules, no build step and no third-party requests: fonts,
images and scripts all load from this site, and nothing tracks visitors.

```
index.html              the home page
privacy/ terms/ docs/   one index.html each; trademarks/ holds the trademark notices
pair/                   for a pairing link opened in a browser: shows the code after the "#", which never
                        reaches a server
404.html                served by Pages for any unknown path
site.css, site.js       the pages' own styles and the home page's script
shared/
  cbase.css, cta.css    base styles and the main button
  app.css, demo.js      the working model of the app in the hero (rooms, the agent working, the approval
                        card with Go ahead / Stop and Undo)
  comic.js              the "A day with it" strip; it shares the hero's answer, so one booking has one answer
  ember.css, ember.js   the living ember: its layers are in ember/, and it glows, blinks, hops and talks
  reveal.js, copy.js    sections that fade in, copy buttons
  doc.js, pair.js       the inner pages' script, and reading a pairing link's code
  fonts/                Instrument Serif and Instrument Sans (Latin subsets) for the pages, each with its
                        licence; Baloo 2 and Figtree come with the shared base styles and aren't used here
  brand/                the app icon and favicon
  logos/                the services' logos, unchanged from each brand's own files (see below)
  theme.js              light paper for everyone; the footer's Dark paper button switches, kept in the browser
_headers                response headers: CSP (scripts from this site only), HSTS, caching
favicon.svg, apple-touch-icon.png, og.png, robots.txt, sitemap.xml
```

## What the page may say

Every claim on the home page describes what the code does today. Things that aren't built yet are marked
"soon" or "planned" (WhatsApp, calendar and iMessage, the Windows app, the relay), and sharing an agent is
marked as an early preview set up through the API. Change the page when those change.

The services are shown with their own logos, taken unchanged from each brand's published files (Google's
PNG icons for Gmail and Google Calendar, SVGs for the rest), only to say what Emberhut works with. Keep them
as the brands ask: never recoloured, filtered, outlined or shadowed; 32 px on desktop and 28 px on phones;
at least half the logo's width clear on every side; the name set as a caption below, never as a lockup.
Apple publishes no iMessage logo for others to use, so iMessage has a plain icon. The trademark notices
are on their own page, `/trademarks/`, linked from every footer; keep it in step with the names and logos
the page uses.

## The paper

The look is paper on a kraft desk, all in CSS: an SVG noise grain on the kraft, a repeating torn-edge mask
on the sheets behind the connectors and the devices, clip-paths for the torn strip and the tape's ends, and
ruled lines on the index card. There are no texture images.

## Caching

Pages link every stylesheet and script with `?v=<version>`, one version for the whole site, and modules
import each other with the same `?v=`. Those files are cached for a year, so after changing any CSS or JS,
put a new version in every `?v=` (pages and imports alike). Art, icons and the ember's layers are cached for
a day; fonts for a year (a new font gets a new file name).

## Motion and accessibility

All motion is CSS and stops under `prefers-reduced-motion`. The pages are light paper whatever the system
setting; the footer's Dark paper button switches to dark (`html[data-theme="dark"]`), and the browser keeps
that choice in its local storage.

## Preview

```sh
python3 -m http.server 8788 --directory site
```

## Deploy

```sh
rsync -a --delete --exclude README.md site/ /tmp/emberhut-site/
wrangler pages deploy /tmp/emberhut-site --project-name emberhut --branch main
```

This leaves out this README, which Pages would otherwise serve at `/README.md`. Set `CLOUDFLARE_API_TOKEN`
(Pages edit) and `CLOUDFLARE_ACCOUNT_ID` first. Any other `--branch` deploys a preview at
`<branch>.emberhut.pages.dev` without touching the live site.
