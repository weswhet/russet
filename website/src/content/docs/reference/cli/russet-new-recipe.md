---
title: russet new-recipe
description: Create a recipe file from a template.
---

`russet new-recipe` creates a recipe file with placeholder values that you
then edit.

## Syntax

```sh
russet new-recipe [OPTIONS] PATH
```

Replace the following:

- `OPTIONS`: any of the options in the following table.
- `PATH`: the path of the recipe file to create, such as
  `MyApp.download.recipe`.

## Description

Russet derives the recipe's `NAME` input from the filename, up to the first
period. For example, `MyApp.download.recipe` gets the name `MyApp`. The
template contains the following keys:

- `Description`: placeholder text.
- `Identifier`: the value of `--identifier`, or `local.NAME`.
- `Input`: a dictionary with the `NAME` value.
- `MinimumVersion`: `1.0` for a property list recipe, or `2.3` for a YAML
  recipe.
- `ParentRecipe`: the value of `--parent-identifier`, if you set it.
- `Process`: one placeholder step named `ProcessorName`.

Russet writes YAML if you pass `--format yaml` or if `PATH` ends in
`.recipe.yaml`. Otherwise, it writes a property list. Russet doesn't check
whether a file already exists at `PATH`, so choose a new path.

If the new recipe is inside one of your search folders, Russet rebuilds the
recipe map.

## Options

| Option | Description |
| --- | --- |
| `--prefs FILE` | Reads preferences from `FILE` in addition to your platform preferences. |
| `-i IDENTIFIER`, `--identifier IDENTIFIER` | Sets the recipe identifier. |
| `-p PARENT_IDENTIFIER`, `--parent-identifier PARENT_IDENTIFIER` | Sets the identifier of the parent recipe. |
| `--format FORMAT` | Writes the recipe as `plist` or `yaml`. The default is `plist`. |

## Exit status

| Status | Meaning |
| --- | --- |
| `0` | Russet saved the recipe. |
| `1` | Russet couldn't write the file. |
| `2` | Russet couldn't parse an option. |
| `255` | You didn't pass exactly one path. On Windows, the status is `-1`. |

## Examples

To create a YAML download recipe, run the following command:

```sh
russet new-recipe --identifier com.example.download.MyApp MyApp.download.recipe.yaml
```

The output starts with the following line:

```text
Saved new recipe to MyApp.download.recipe.yaml
```

## Related pages

- [Processors](/reference/processors/)
- [`russet audit`](/reference/cli/russet-audit/)
