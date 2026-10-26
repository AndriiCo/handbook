// Public shell is a client-rendered SPA served by axum from /apostille.
// No prerendered pages, so adapter-static's `index.html` fallback never
// collides with a prerendered homepage.
export const prerender = false;
export const ssr = false;
