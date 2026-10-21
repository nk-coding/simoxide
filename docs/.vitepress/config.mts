import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'SimOxide',
  description:
    'A Rust simulator for Palladio models, compatible with SimuLizar 5.2.2 and Slingshot, with a QVTo engine',
  // GitHub Pages serves the site under /simoxide/ (set by .github/workflows/docs.yml)
  base: process.env.DOCS_BASE ?? '/',
  cleanUrls: true,
  lastUpdated: false,
  markdown: { math: false },
  themeConfig: {
    nav: [
      { text: 'Guide', link: '/guide/introduction' },
      { text: 'Modes', link: '/guide/modes' },
      { text: 'Semantics', link: '/spec/' },
      { text: 'Correctness', link: '/correctness/' },
      { text: 'Performance', link: '/performance/' },
      { text: 'QVTo', link: '/qvto/' },
      {
        text: 'More',
        items: [
          { text: 'Slingshot semantics', link: '/spec/slingshot/' },
          { text: 'Reference simulators', link: '/reference-simulator/refsim' },
          { text: 'Licensing', link: '/guide/licensing' },
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
            { text: 'SimOxide licensing', link: '/guide/licensing' },
            { text: 'Contributing', link: '/development/contributing' },
          ],
        },
      ],
      '/': [
        {
          text: 'Start',
          items: [
            { text: 'Introduction', link: '/guide/introduction' },
            { text: 'Getting started', link: '/guide/getting-started' },
            { text: 'Modes and features', link: '/guide/modes' },
          ],
        },
        {
          text: 'Simulation',
          items: [
            { text: 'Supported models', link: '/guide/scope' },
            { text: 'Fast mode', link: '/guide/fast-mode' },
            { text: 'Slingshot-compatible mode', link: '/guide/slingshot' },
            { text: 'Load profiles', link: '/guide/load-profile' },
          ],
        },
        {
          text: 'Runtime changes',
          items: [
            { text: 'Reconfiguration rules', link: '/guide/reconfiguration' },
            { text: 'Scaling policies (SPD)', link: '/guide/spd' },
            { text: 'Autoscaling (HPA)', link: '/guide/hpa' },
            { text: 'Snapshots, forks, changes', link: '/guide/snapshots' },
          ],
        },
        {
          text: 'Interfaces',
          items: [
            { text: 'Command line', link: '/guide/cli' },
            { text: 'Library API', link: '/guide/library' },
            { text: 'Output formats', link: '/guide/formats' },
            { text: 'Snapshot format', link: '/formats/snapshot' },
            { text: 'QVTo engine', link: '/qvto/' },
          ],
        },
        {
          text: 'Licensing',
          items: [{ text: 'Licensing and third party', link: '/guide/licensing' }],
        },
        {
          text: 'Performance',
          collapsed: true,
          items: [
            { text: 'Overview', link: '/performance/' },
            { text: 'Comparison in detail', link: '/performance/comparison' },
            { text: 'Engine performance', link: '/performance/engine' },
          ],
        },
        {
          text: 'Correctness',
          collapsed: true,
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
          text: 'Reference simulators',
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
