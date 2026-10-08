---
title: russet processor-info
description: Show a processor's description, input variables, and output variables.
---

`russet processor-info` prints the description, input variables, and output
variables of one processor.

## Syntax

```sh
russet processor-info [OPTIONS] PROCESSOR
```

Replace the following:

- `OPTIONS`: any of the options in the following table.
- `PROCESSOR`: a processor name from `russet list-processors`, such as
  `URLDownloader`.

## Description

Russet prints the processor's `Description`, its `Input variables` with
whether the processor requires each one, its default and description, and its
`Output variables`.

Some defaults depend on the platform:

- `Unarchiver` shows `USE_PYTHON_NATIVE_EXTRACTOR` as `False` on macOS and
  `True` on Linux and Windows.
- `SignToolVerifier` shows the `signtool_path` that Russet finds on the
  current computer, or `None`.

## Options

| Option | Description |
| --- | --- |
| `--prefs FILE` | Reads preferences from `FILE` in addition to your platform preferences. |
| `-r RECIPE`, `--recipe RECIPE` | Accepted for compatibility. Russet ignores it. |
| `-d FOLDER`, `--search-dir FOLDER` | Accepted for compatibility. Russet ignores it. |
| `--override-dir FOLDER` | Accepted for compatibility. Russet ignores it. |

## Exit status

| Status | Meaning |
| --- | --- |
| `0` | Russet printed the information. |
| `1` | Russet couldn't read the `--prefs` file. |
| `2` | Russet couldn't parse an option. |
| `255` | You didn't name exactly one processor, or Russet doesn't implement the processor. On Windows, the status is `-1`. |

## Examples

To show the variables of `URLDownloader`, run the following command:

```sh
russet processor-info URLDownloader
```

The output starts like the following:

```text
Description: Downloads a URL to the specified download_dir using curl.
Input variables:
   url:
     required: True
     description: The URL to download.
```

## Related pages

- [Processors](/reference/processors/)
- [`russet list-processors`](/reference/cli/russet-list-processors/)
