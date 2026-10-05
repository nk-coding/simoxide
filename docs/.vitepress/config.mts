import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'SimOxide',
  description: 'A byte-exact, much faster Rust port of the Palladio simulator SimuLizar 5.2.2',
  // GitHub Pages serves the site under /simoxide/ (set by .github/workflows/docs.yml)
  base: process.env.DOCS_BASE ?? '/',
  cleanUrls: true,
  lastUpdated: false,
  markdown: { math: false },
  themeConfig: {
    nav: [
      { text: 'Guide', link: '/guide/introduction' },
      { text: 'Performance', link: '/performance/' },
      { text: 'Correctness', link: '/correctness/' },
      { text: 'Semantics', link: '/spec/' },
      {
        text: 'More',
        items: [
          { text: 'Reference simulator', link: '/reference-simulator/refsim' },
          { text: 'Development', link: '/development/contributing' },
        ],
      },
    ],
    sidebar: [
      {
        text: 'Guide',
        items: [
          { text: 'Introduction', link: '/guide/introduction' },
          { text: 'Getting started', link: '/guide/getting-started' },
          { text: 'Command line', link: '/guide/cli' },
          { text: 'Library API', link: '/guide/library' },
          { text: 'Fast mode', link: '/guide/fast-mode' },
          { text: 'Supported models', link: '/guide/scope' },
          { text: 'Output formats', link: '/guide/formats' },
        ],
      },
      {
        text: 'Performance',
        items: [
          { text: 'Overview', link: '/performance/' },
          { text: 'Comparison in detail', link: '/performance/comparison' },
          { text: 'Engine performance', link: '/performance/engine' },
        ],
      },
      {
        text: 'Correctness',
        items: [
          { text: 'Exactness and evidence', link: '/correctness/' },
          { text: 'Testing', link: '/correctness/testing' },
          { text: 'Deviations', link: '/correctness/deviations' },
          { text: 'Reference bugs', link: '/correctness/reference-bugs' },
        ],
      },
      {
        text: 'Semantics (SimuLizar 5.2.2)',
        collapsed: true,
        items: [
          { text: 'Overview', link: '/spec/' },
          { text: 'Simulation core', link: '/spec/simulation' },
          { text: 'Workloads', link: '/spec/workloads' },
          { text: 'Actions', link: '/spec/actions' },
          { text: 'Measurements', link: '/spec/measurements' },
          { text: 'Stochastic expressions', link: '/spec/stoex' },
          { text: 'Random numbers', link: '/spec/random' },
          { text: 'Schedulers', link: '/spec/scheduler' },
        ],
      },
      {
        text: 'Reference simulator',
        collapsed: true,
        items: [
          { text: 'refsim', link: '/reference-simulator/refsim' },
          { text: 'Patches', link: '/reference-simulator/patches' },
          { text: 'Model corpus', link: '/reference-simulator/corpus' },
        ],
      },
      {
        text: 'Development',
        collapsed: true,
        items: [
          { text: 'Contributing', link: '/development/contributing' },
          { text: 'Benchmarking', link: '/development/benchmarking' },
          { text: 'Test kit', link: '/development/testkit' },
        ],
      },
    ],
    socialLinks: [{ icon: 'github', link: 'https://github.com/nk-coding/simoxide' }],
    editLink: {
      pattern: 'https://github.com/nk-coding/simoxide/edit/main/docs/:path',
      text: 'Edit this page on GitHub',
    },
    outline: { level: [2, 3] },
    search: { provider: 'local' },
    footer: {
      message: 'Released under the Eclipse Public License 2.0.',
    },
  },
})
