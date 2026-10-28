import { defineConfig } from "cf/config";

// An assets-only Worker serving the Trunk build in dist/ (see wrangler.config.ts).
// Deploying attaches the custom domain, which creates its DNS record.
export default defineConfig({
	accountId: "840e4653bf430c9c6a6e29441f34df77",
	worker: {
		name: "delphi-dash",
		compatibilityDate: "2026-10-01",
		domains: ["delphi.openlabs.lat"],
	},
});
