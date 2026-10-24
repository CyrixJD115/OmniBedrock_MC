// The panel is a client-rendered SPA: every page fetches from the API after
// load, so there is nothing to prerender or server-render. The static adapter
// emits a bare index.html fallback that the Rust binary serves for all routes.
export const ssr = false;
export const prerender = false;
