// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// A github.io project site, so the pages are served under a base path. Moving
// to a domain of its own means a CNAME in ./public, `site` set to that domain
// and no `base` at all - the two arrangements do not mix.
export default defineConfig({
	site: 'https://lacodda.github.io',
	base: '/kasl-server',
	integrations: [
		starlight({
			title: 'kasl-server',
			description:
				'Team server for kasl: collects work-time data from employees’ kasl agents and turns it into dashboards, reports, and personal pages.',
			logo: {
				src: './src/assets/logo.svg',
				alt: 'kasl-server',
			},
			favicon: '/favicon.svg',
			customCss: ['./src/styles/brand.css'],
			social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/lacodda/kasl-server' }],
			editLink: {
				baseUrl: 'https://github.com/lacodda/kasl-server/edit/main/docs/',
			},
			sidebar: [
				{ label: 'Getting Started', slug: 'getting-started' },
				{
					label: 'Guides',
					items: [{ autogenerate: { directory: 'guides' } }],
				},
				{
					label: 'Reference',
					items: [{ autogenerate: { directory: 'reference' } }],
				},
				{
					label: 'Concepts',
					items: [{ autogenerate: { directory: 'concepts' } }],
				},
			],
		}),
	],
});
