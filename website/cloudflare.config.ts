import { defineConfig } from 'cf/config';

export default defineConfig({
	worker: {
		name: 'dupin-website',
		compatibilityDate: '2026-11-21',
		domains: ['dupin.app'],
		assets: {
			htmlHandling: 'auto-trailing-slash',
			notFoundHandling: '404-page',
		},
	},
});
