import { cloudflare } from '@cloudflare/vite-plugin';
import { defineConfig, runnerImport } from 'vite';
import { oxContent } from '@ox-content/vite-plugin';
import { cloudflareRedirect } from '@ryoppippi/vite-plugin-cloudflare-redirect';
import type * as Theme from './theme/index.ts';
import { repo } from './theme/site.ts';

// ox-content writes pages to its own outDir instead of following the Vite environment's, which the
// Cloudflare plugin moves into Build Output, so the pages must land where cf deploys assets from.
const assetsDir = '.cloudflare/output/v1/workers/default/assets';

// The config bundler ignores tsconfig, so the JSX theme goes through Vite's module runner instead.
const {
	module: { theme },
} = await runnerImport<typeof Theme>('./theme/index.ts', { configFile: false });

export default defineConfig({
	plugins: [
		cloudflare(),
		cloudflareRedirect({
			mode: 'generate',
			entries: [
				{ from: '/gh', to: repo, status: 302 },
				{ from: '/gh/*', to: `${repo}/:splat`, status: 302 },
			],
		}),
		oxContent({
			srcDir: 'content',
			outDir: assetsDir,
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
