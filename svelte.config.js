// Read by `svelte-check`, which type-checks `.svelte` files.
//
// Without this file there was no such check at all: `npm run build` compiles components without
// type-checking them, `tsc` does not read `.svelte`, and `eslint` parses them without following
// types. A deleted `$props()` call therefore reached a running window as an empty `<body>` with
// every gate green -- which is what prompted this file.
//
// `vitePreprocess` rather than a hand-written preprocessor: it reuses the same transform the
// build already applies, so what is checked is what ships. A second preprocessor configured
// separately would be a second thing to keep in step.
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

export default {
  preprocess: vitePreprocess(),
};
