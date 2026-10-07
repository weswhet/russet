// @ts-check
import { defineConfig } from 'astro/config';
import { satteri } from '@astrojs/markdown-satteri';
import starlight from '@astrojs/starlight';
import { baseLinks } from './src/plugins/base-links.mjs';

// Set RUSSET_DOCS_SITE to the public origin when you build for a deployment, so
// canonical links and the sitemap use it. Set RUSSET_DOCS_BASE when the site is
// served from a subfolder: the GitHub Pages workflow sets `/russet`.
// scripts/serve-tailnet.sh serves from the root of this computer's tailnet name.
const base = process.env.RUSSET_DOCS_BASE || '/';

export default defineConfig({
	site: process.env.RUSSET_DOCS_SITE,
	base,
	telemetry: false,
	markdown: {
		processor: satteri({ hastPlugins: [baseLinks(base)] }),
	},
	integrations: [
		starlight({
			title: 'Russet',
			description:
				'Documentation for Russet, a native implementation of AutoPkg that runs your recipes on macOS, Linux, and Windows without Python.',
			favicon: '/favicon.svg',
			customCss: ['./src/styles/russet.css'],
			social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/weswhet/russet' }],
			lastUpdated: false,
			pagination: true,
			tableOfContents: { minHeadingLevel: 2, maxHeadingLevel: 3 },
			sidebar: [
				{ label: 'Overview', link: '/' },
				{
					label: 'Get started',
					items: ['get-started/requirements', 'get-started/install', 'get-started/quickstart'],
				},
				{
					label: 'Concepts',
					items: [
						'concepts/how-russet-works',
						'concepts/compatibility',
						'concepts/platform-support',
						'concepts/native-apple-formats',
						'concepts/recipe-trust',
					],
				},
				{
					label: 'Work with recipes',
					items: [
						'guides/add-recipe-repositories',
						'guides/run-recipes',
						'guides/create-overrides',
						'guides/audit-recipes',
						'guides/schedule-runs',
					],
				},
				{
					label: 'Munki',
					items: ['guides/import-into-munki'],
				},
				{
					label: 'Administration',
					items: [
						'guides/configure-preferences',
						'guides/switch-from-autopkg',
						'guides/roll-back',
					],
				},
				{
					label: 'Reference',
					items: [
						{
							label: 'Command-line reference',
							collapsed: true,
							items: [{ autogenerate: { directory: 'reference/cli' } }],
						},
						'reference/processors',
						'reference/preferences',
						'reference/environment-variables',
						'reference/files-and-paths',
						'reference/exit-codes',
					],
				},
				{
					label: 'Resources',
					items: [
						'resources/troubleshooting',
						'resources/glossary',
						'resources/release-notes',
						'resources/about-these-docs',
					],
				},
			],
		}),
	],
});
