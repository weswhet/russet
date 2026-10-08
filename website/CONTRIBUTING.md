# Contributing to the Russet documentation

The Russet documentation follows the
[Google developer documentation style guide](https://developers.google.com/style).
The Google guide is the authority for every page on this site. This file
summarizes the rules that come up most often in these pages. When this file and
the Google guide disagree, follow the Google guide and fix this file.

## Run the site locally

```sh
cd website
npm install
npm run dev        # http://localhost:4321, reloads on save
npm run build      # static output in website/dist
npm run preview    # serve the built site locally
npm run lint:style # check prose against the Google style rules with Vale
npm run tailnet    # build and serve to your tailnet at https://HOST.TAILNET.ts.net/
npm run tailnet:stop
```

`npm run tailnet` builds the site for this computer's MagicDNS name, runs a
static server on `127.0.0.1:4321`, and points `tailscale serve` at it. You can
open either `https://HOST.TAILNET.ts.net/` or `http://TAILSCALE_IP/`. The site
is reachable only from your tailnet (it doesn't use Funnel). The preview server
doesn't survive a restart, so run `npm run tailnet` again after you log in.

`astro.config.mjs` reads the site's public origin from `RUSSET_DOCS_SITE` and
its base path from `RUSSET_DOCS_BASE`. Set both when you build for a
deployment so that links, canonical URLs, and the sitemap use the right
address. To build the site the way GitHub Pages serves it, run:

```sh
RUSSET_DOCS_SITE=https://weswhet.github.io RUSSET_DOCS_BASE=/russet npm run build
```

## Publish the site

The `Docs` workflow in `.github/workflows/docs.yml` builds the site for every
pull request that changes `website/`. When a change to `website/` reaches the
`main` branch, the workflow publishes the site to GitHub Pages at
<https://weswhet.github.io/russet/>. To publish again without a change, run the
workflow from the **Actions** tab.

## Check style with Vale

`npm run lint:style` downloads the Google style package and checks every page.
Fix every error and warning before you merge a change. Treat a suggestion as a
prompt to reread the sentence; fix it unless the rule doesn't apply, such as a
passive sentence where the actor is unknown.

Add product names, command names, and other correct terms that Vale flags as
misspelled to `styles/config/vocabularies/Russet/accept.txt`. Don't add a word
to silence a rule that the sentence actually breaks.

Vale is an optional dependency because its installer downloads the Vale binary
from GitHub. If that download fails, `npm install` and the site build still
succeed.

## Page types

Every page is one of these types. Keep types separate: a how-to guide links to
a concept instead of explaining it at length.

| Type | Purpose | Title form | Location |
| --- | --- | --- | --- |
| Overview | What Russet is and where to start. | Noun phrase | `index.mdx` |
| Get started | Requirements, installation, and a first task. | Noun phrase or verb | `get-started/` |
| Concept | How something works and why. | Noun phrase, such as "Recipe trust" | `concepts/` |
| How-to guide | Steps to complete one task. | Bare infinitive, such as "Run recipes" | `guides/` |
| Reference | Facts to look up: commands, keys, codes. | Noun phrase | `reference/` |
| Troubleshooting | Symptom, cause, and resolution. | Noun phrase | `resources/` |

### How-to guide structure

1. A one- or two-sentence introduction that says what the reader accomplishes.
2. `## Before you begin`: prerequisites as a bulleted list.
3. One `##` section per task, titled with a bare infinitive ("Add a recipe
   repository"). Each task starts with a sentence that states the goal,
   followed by numbered steps when there's more than one action.
4. `## What's next`: a bulleted list of related pages.

## Voice and tone

- Address the reader as "you". Use the imperative for instructions.
- Use present tense. Write "Russet verifies", not "Russet will verify".
- Use active voice. Make clear who or what performs the action.
- Be conversational but precise. Don't use "please", "simply", "just",
  "easy", or "obviously".
- Don't use "we" to mean the Russet project. Name the product instead.
- Don't use Latin abbreviations. Write "for example", "that is", and "and so
  on" instead of "e.g.", "i.e.", and "etc.".
- Put conditions before instructions: "To check a recipe's trust, run ...",
  "If verification fails, run ...".
- Spell out an abbreviation the first time that a page uses it, such as
  "continuous integration (CI)".

## Formatting

- Use sentence case for titles and headings. Don't end headings with
  punctuation.
- Use numbered lists for sequential steps and bulleted lists for everything
  else. Introduce every list with a complete sentence that ends in a colon.
- Use the serial (Oxford) comma.
- Put commands, flags, file names, paths, preference keys, environment
  variables, processor names, recipe identifiers, literal values, and output
  in code font. Don't use code font for product names such as Russet,
  AutoPkg, Munki, or macOS.
- Write placeholders in uppercase with underscores, such as `RECIPE_NAME` and
  `REPOSITORY_URL`. After a code sample that contains placeholders, explain
  each one in a "Replace the following:" list.
- Give code blocks a language (`sh`, `powershell`, `text`, `xml`, `yaml`,
  `json`). Don't include a shell prompt (`$`) in commands that the reader
  copies. Show output in a separate `text` block.
- Use the Starlight asides for notices, and use them sparingly:
  `:::note` for useful extra information, `:::caution` for possible data loss
  or a surprising result, and `:::danger` for irreversible harm.
- Use descriptive link text. Link to the page title or describe the
  destination; never write "click here" or "this page".
- Link to another page with a path from the site root, such as
  `/guides/run-recipes/`. The build adds the base path to Markdown links. In
  frontmatter and in MDX component props, such as a `LinkCard` `href`, the
  build doesn't add it, so use a path relative to the page instead.
- Write dates as "October 7, 2026". Include a time zone when the time matters.

## Word list

| Use | Don't use |
| --- | --- |
| the `russet` command | the Russet binary, the `autopkg` command (except when you contrast Russet with Python AutoPkg) |
| AutoPkg, Python AutoPkg (the original project, when you contrast it with Russet) | the old AutoPkg, legacy AutoPkg |
| recipe, parent recipe, override | script, job |
| processor, built-in processor | step, plug-in |
| custom processor (a processor that a recipe repository supplies in Python) | third-party processor |
| recipe repository | recipe repo (except in command names such as `repo-add`) |
| trust information | trust info (except in command names and keys) |
| Munki repository | Munki repo (except in preference keys such as `MUNKI_REPO`) |
| preference, preference key | setting, default (for a preference key) |
| Apple silicon, Intel | M-series, x86 (in prose) |
| select | click on, hit |
| turn on, turn off (a setting) | enable, disable (except for command or key names) |

## Accuracy rules for Russet

- Describe the behavior of the current source on `main`. When the
  engineering records in `compatibility/` or in the
  [russet-compat](https://github.com/weswhet/russet-compat) repository
  disagree with the source, the source wins.
- Russet doesn't have a published release yet, and 0.1.0 will be the first.
  Readers install Russet by building it from source and packaging it with
  `cargo xtask package`. Don't describe prebuilt archives, a Homebrew formula,
  an installer package, or notarized downloads as available.
- `russet version` reports the AutoPkg compatibility version (3.0.0), not the
  Russet distribution version. Don't confuse the two.
- Don't document hidden or internal commands, internal environment variables,
  or test-only behavior that a user can't act on.
- Never include real credentials, host names, serial numbers, or tokens. Use
  clearly fake values in examples.
