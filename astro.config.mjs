// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import consoleGrammar from './src/shiki/console-grammar.mjs';

const base = '/cadence';

// https://astro.build/config
export default defineConfig({
	site: 'https://pratirupm.github.io',
	base,
	markdown: {
		shikiConfig: {
			langs: [consoleGrammar],
			langAlias: {
				console: 'book-console',
			},
		},
	},
	integrations: [
		starlight({
			title: 'Cadence',
			description: 'Build a multi-user task service in Rust, from CLI to production.',
			customCss: ['./src/styles/fonts.css', './src/styles/custom.css'],
			head: [
				{
					tag: 'script',
					attrs: { src: `${base}/console-colors.js`, defer: true },
				},
				{
					tag: 'link',
					attrs: {
						rel: 'preload',
						href: `${base}/fonts/0xProto-Regular.woff2`,
						as: 'font',
						type: 'font/woff2',
						crossorigin: 'anonymous',
					},
				},
				{
					tag: 'link',
					attrs: {
						rel: 'preload',
						href: `${base}/fonts/0xProto-Bold.woff2`,
						as: 'font',
						type: 'font/woff2',
						crossorigin: 'anonymous',
					},
				},
			],
			lastUpdated: true,
			sidebar: [
				{
					label: 'Start here',
					items: [
						{ label: 'Why this book', slug: 'start/why-this-book' },
						{ label: 'What you will build', slug: 'start/what-you-will-build' },
						{ label: 'How to use this book', slug: 'start/how-to-use' },
						{ label: 'Set up your workbench', slug: 'start/setup' },
					],
				},
				{
					label: 'Part I — From CLI to service',
					items: [
						{ label: 'The sequel', slug: 'part-1/the-sequel' },
						{ label: 'SQLite and migrations', slug: 'part-1/sqlite-and-migrations' },
						{ label: 'Accounts', slug: 'part-1/accounts' },
						{ label: 'Workspaces and projects', slug: 'part-1/workspaces-and-projects' },
						{ label: 'Tasks', slug: 'part-1/tasks' },
						{ label: 'The MPA', slug: 'part-1/the-mpa' },
					],
				},
				{
					label: 'Part II — The task engine',
					items: [
						{ label: 'Query DSL', slug: 'part-2/query-dsl' },
						{ label: 'Recurrence', slug: 'part-2/recurrence' },
						{ label: 'The next occurrence', slug: 'part-2/next-occurrence' },
						{ label: 'Reminders', slug: 'part-2/reminders' },
						{ label: 'Notifications', slug: 'part-2/notifications' },
						{ label: 'Sharing deep', slug: 'part-2/sharing-deep' },
						{ label: 'The API', slug: 'part-2/api' },
					],
				},
				{
					label: 'Part III — Sync and real time',
					items: [
						{ label: 'The change log', slug: 'part-3/change-log' },
						{ label: 'The CLI sync client', slug: 'part-3/cli-sync' },
						{ label: 'Conflicts', slug: 'part-3/conflicts' },
						{ label: 'Server-sent events', slug: 'part-3/sse' },
						{ label: 'Live UI', slug: 'part-3/live-ui' },
					],
				},
				{
					label: 'Part IV — Production',
					items: [
						{ label: 'Webhooks', slug: 'part-4/webhooks' },
						{ label: 'Observability', slug: 'part-4/observability' },
						{ label: 'Security', slug: 'part-4/security' },
						{ label: 'Performance', slug: 'part-4/performance' },
						{ label: 'Deployment', slug: 'part-4/deployment' },
						{ label: 'Capstone', slug: 'part-4/capstone' },
					],
				},
				{
					label: 'Appendices',
					items: [
						{ label: 'Book one recap', slug: 'appendix/book-one-recap' },
						{ label: 'Architecture map', slug: 'appendix/architecture' },
						{ label: 'Command reference', slug: 'appendix/commands' },
						{ label: 'Glossary', slug: 'appendix/glossary' },
						{ label: 'Where next', slug: 'appendix/where-next' },
					],
				},
			],
		}),
	],
});
