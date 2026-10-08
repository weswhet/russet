---
title: Glossary
description: Definitions of the terms that the Russet documentation uses.
---

This page defines the terms that the Russet documentation uses.

## AutoPkg

The open source tool, written in Python, that runs recipes to download,
package, and import software. The documentation calls it Python AutoPkg when
it contrasts AutoPkg with Russet.

## Built-in processor

A processor that's part of Russet itself. Russet implements AutoPkg's built-in
processors and some processors that recipe repositories commonly supply. For
the list, see [Processors](/reference/processors/).

## Cache

The folder where Russet keeps downloads, build products, receipts, and run
results. By default, it's `~/Library/AutoPkg/Cache`.

## Catalog

A Munki file that lists the items that a group of Munki clients can install,
such as `testing` or `production`.

## Check phase

The part of a recipe before its `EndOfCheckPhase` step. `russet run --check`
runs only this part, which usually checks for and downloads a new version.

## Compatibility version

The AutoPkg version that Russet implements, 3.0.0. `russet version` prints
it, and recipes compare their `MinimumVersion` with it.

## Custom processor

A processor that a recipe repository supplies as a Python file. Russet
doesn't run custom processors.

## Distribution version

The version of a Russet release, such as 0.1.0. A release archive records it
in `RELEASE.json`. An archive that you build from source doesn't have one.

## Helper services

Two launchd services on macOS, `russet-server` and `russet-installd`, that
run as root to build packages and install software for recipes. launchd starts
them as `russet --server` and `russet --installd`.

## Munki

An open source tool for managing software on Mac computers. Russet imports
software into Munki repositories.

## Munki repository

The folder that Munki clients download software and metadata from. Russet
supports file-based Munki repositories only.

## Override

A recipe in your overrides folder that names a parent recipe, changes its
input values, and records its trust information.

## Parent recipe

A recipe that another recipe builds on. A child recipe inherits its parent's
input variables and processors and adds its own.

## Pkginfo file

A Munki file that describes one version of one item, such as its name,
version, installer item, and catalogs.

## Processor

A step in a recipe that does one task, such as downloading a file or
verifying a code signature.

## Receipt

A property list that Russet writes after each recipe runs. It records the
recipe's input and each processor's input and output.

## Recipe

A property list or YAML file that describes how to download, package, or
import a piece of software. A recipe lists processors and their inputs.

## Recipe list

A file that names recipes to run in one command, one per line. A recipe list
can also be a property list.

## Recipe map

A file that indexes the recipes in your search folders by name and
identifier. By default, it's `~/Library/AutoPkg/recipe_map.json`.

## Recipe repository

A Git repository of recipes. `russet repo-add` clones recipe repositories
into `~/Library/AutoPkg/RecipeRepos` by default.

## Rollback generation

A folder that an installer creates for each installation. It holds the files
that the installation replaced, so that a rollback can restore them.

## Trust information

The `ParentRecipeTrustInfo` dictionary in an override. It records hashes of
the parent recipes and related files, so that Russet can detect changes. For
details, see [Recipe trust](/concepts/recipe-trust/).
