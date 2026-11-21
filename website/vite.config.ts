import { defineConfig } from 'vite';
import { oxContent } from '@ox-content/vite-plugin';
import { cloudflareRedirect } from '@ryoppippi/vite-plugin-cloudflare-redirect';
import { theme } from './theme/index.ts';
import { repo } from './theme/site.ts';

export default defineConfig({
	plugins: [
		cloudflareRedirect({
			mode: 'generate',
			entries: [
				{ from: '/gh', to: repo, status: 302 },
				{ from: '/gh/*', to: `${repo}/:splat`, status: 302 },
			],
		}),
		oxContent({
			srcDir: 'content',
			outDir: 'dist',
			highlight: true,
			search: false,
			docs: false,
			ssg: {
				siteName: 'Dupin',
				siteUrl: 'https://dupin.app',
				minifyHtml: true,
				render: theme,
			},
		}),
	],
});
