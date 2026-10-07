---
title: About these docs
description: The structure of the Russet documentation, the style guide that it follows, and how to build it locally.
---

This page explains the structure of the Russet documentation and how to work
on it.

## Documentation structure

The documentation separates pages by what you're trying to do:

- **Get started** pages help you install Russet and run your first recipe.
- **Concept** pages explain how Russet works and why it behaves the way it
  does.
- **How-to guides** give you the steps to complete one task.
- **Reference** pages list facts to look up, such as commands, processors,
  preference keys, and exit codes.
- **Resources** include troubleshooting, a glossary, and release notes.

## Style

The documentation follows the
[Google developer documentation style guide](https://developers.google.com/style).
The repository file `website/CONTRIBUTING.md` summarizes the rules that these
pages use most, including page structure, voice, formatting, and a word list.

The site checks prose with [Vale](https://vale.sh/) and the Google style
package.

## Build the site locally

The site uses [Starlight](https://starlight.astro.build/). To build and view
the site on your computer, follow these steps:

1. In a terminal, go to the `website` directory of the Russet repository.
1. Install the dependencies:

   ```sh
   npm install
   ```

1. Start the development server:

   ```sh
   npm run dev
   ```

1. Open `http://localhost:4321` in a browser.

To check the prose against the style rules, run `npm run lint:style`.
