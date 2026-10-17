import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'SimOxide',
  description:
    'A byte-exact, much faster Rust port of the Palladio simulator SimuLizar 5.2.2, with a QVTo engine',
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
      { text: 'QVTo', link: '/qvto/' },
      {
        text: 'More',
        items: [
          { text: 'Reference simulator', link: '/reference-simulator/refsim' },
          { text: 'Development', link: '/development/contributing' },
        ],
      },
    ],
    sidebar: {
      '/qvto/': [
        {
          text: 'QVTo engine',
          items: [
            { text: 'Overview', link: '/qvto/' },
            { text: 'Usage', link: '/qvto/usage' },
            { text: 'Architecture', link: '/qvto/architecture' },
            { text: 'Conformance', link: '/qvto/conformance' },
            { text: 'Performance', link: '/qvto/performance' },
            { text: 'QVTo in SimOxide', link: '/qvto/simoxide' },
            { text: 'Licensing', link: '/qvto/licensing' },
            { text: 'Development', link: '/qvto/development' },
          ],
        },
        {
          text: 'Related',
          items: [
            { text: 'Foundation crates', link: '/development/foundation' },
            { text: 'Reconfiguration rules', link: '/guide/reconfiguration' },
            { text: 'Scaling policies (SPD)', link: '/guide/spd' },
            { text: 'Contributing', link: '/development/contributing' },
          ],
        },
      ],
      '/': [
        {
          text: 'Guide',
          items: [
            { text: 'Introduction', link: '/guide/introduction' },
            { text: 'Getting started', link: '/guide/getting-started' },
            { text: 'Command line', link: '/guide/cli' },
            { text: 'Library API', link: '/guide/library' },
            { text: 'Snapshots', link: '/guide/snapshots' },
            { text: 'Reconfiguration rules', link: '/guide/reconfiguration' },
            { text: 'Scaling policies (SPD)', link: '/guide/spd' },
            { text: 'Autoscaling (HPA)', link: '/guide/hpa' },
            { text: 'Fast mode', link: '/guide/fast-mode' },
            { text: 'Slingshot flavour', link: '/guide/slingshot' },
            { text: 'Supported models', link: '/guide/scope' },
            { text: 'Output formats', link: '/guide/formats' },
            { text: 'Snapshot format', link: '/formats/snapshot' },
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
            { text: 'Slingshot flavour', link: '/correctness/slingshot' },
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
            { text: 'Model changes', link: '/spec/model-changes' },
            { text: 'HPA autoscaling', link: '/spec/hpa' },
          ],
        },
        {
          text: 'Semantics (Slingshot)',
          collapsed: true,
          items: [
            { text: 'Overview', link: '/spec/slingshot/' },
            { text: 'Event core', link: '/spec/slingshot/core' },
            { text: 'Usage model', link: '/spec/slingshot/usage' },
            { text: 'SEFF interpretation', link: '/spec/slingshot/seff' },
            { text: 'Resources', link: '/spec/slingshot/resources' },
            { text: 'Monitoring', link: '/spec/slingshot/monitoring' },
            { text: 'Random numbers', link: '/spec/slingshot/random' },
            { text: 'Scaling policies (SPD)', link: '/spec/slingshot/spd' },
            { text: 'Quirks', link: '/spec/slingshot/quirks' },
            { text: 'Exact comparison', link: '/spec/slingshot/exact' },
          ],
        },
        {
          text: 'Reference simulator',
          collapsed: true,
          items: [
            { text: 'refsim', link: '/reference-simulator/refsim' },
            { text: 'Patches', link: '/reference-simulator/patches' },
            { text: 'Slingshot runner', link: '/reference-simulator/slingshot' },
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
            { text: 'Foundation crates', link: '/development/foundation' },
          ],
        },
        {
          text: 'QVTo engine',
          collapsed: true,
          items: [
            { text: 'Overview', link: '/qvto/' },
            { text: 'QVTo in SimOxide', link: '/qvto/simoxide' },
          ],
        },
      ],
    },
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
