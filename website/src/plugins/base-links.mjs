// Starlight adds the site's base path to its own navigation, but not to links
// inside page content. This Sätteri plugin adds it to root-relative links in
// Markdown and MDX, so pages can link to `/guides/run-recipes/` whether the
// site is served from `/` or from a subfolder such as `/russet/` on GitHub
// Pages. It doesn't change component props: link to pages from an MDX
// component's `href` with a path relative to the page instead.
export function baseLinks(base = '/') {
	const prefix = base.replace(/\/+$/, '');
	if (!prefix) return null;
	return {
		name: 'russet-base-links',
		element: {
			filter: ['a'],
			visit(node, ctx) {
				const href = node.properties?.href;
				if (
					typeof href === 'string' &&
					href.startsWith('/') &&
					!href.startsWith('//') &&
					href !== prefix &&
					!href.startsWith(`${prefix}/`)
				) {
					ctx.setProperty(node, 'href', `${prefix}${href}`);
				}
			},
		},
	};
}
