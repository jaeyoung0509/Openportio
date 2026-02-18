import { defineConfig } from 'vitepress';

const base = process.env.VITEPRESS_BASE || '/';
const siteOrigin = (process.env.VITEPRESS_SITE_ORIGIN || 'https://jaeyoung0509.github.io').replace(
  /\/$/,
  ''
);
const basePath = base === '/' ? '' : base.replace(/\/$/, '');

export default defineConfig({
  base,
  title: 'Openportio',
  description: 'Single-port REST + gRPC server framework for Rust.',
  cleanUrls: true,
  lastUpdated: true,
  lang: 'en-US',
  sitemap: {
    hostname: `${siteOrigin}${basePath}`
  },
  themeConfig: {
    logo: '/openportio-logo.svg',
    siteTitle: 'Openportio Docs',
    nav: [
      { text: 'Getting Started', link: '/getting-started' },
      { text: 'REST + gRPC', link: '/guides/rest-grpc' },
      { text: 'REST DX', link: '/guides/rest-fastapi-dx' },
      { text: 'gRPC DX', link: '/guides/grpc-fastapi-dx' },
      { text: 'DX / DTO', link: '/guides/dx-builder-dto' },
      { text: 'Production', link: '/production/deployment' },
      { text: 'Failure Drills', link: '/production/failure-modes' },
      { text: 'Contracts', link: '/reference/contracts' }
    ],
    sidebar: [
      {
        text: 'Start Here',
        items: [
          { text: 'Overview', link: '/' },
          { text: 'Getting Started', link: '/getting-started' }
        ]
      },
      {
        text: 'Guides',
        items: [
          { text: 'REST + gRPC Runtime', link: '/guides/rest-grpc' },
          { text: 'REST FastAPI-like DX', link: '/guides/rest-fastapi-dx' },
          { text: 'gRPC FastAPI-like DX', link: '/guides/grpc-fastapi-dx' },
          { text: 'Builder / DTO / Validation', link: '/guides/dx-builder-dto' }
        ]
      },
      {
        text: 'Production',
        items: [
          { text: 'Deployment Checklist', link: '/production/deployment' },
          { text: 'Failure-Mode Drills', link: '/production/failure-modes' }
        ]
      },
      {
        text: 'Reference',
        items: [{ text: 'Contract Artifacts', link: '/reference/contracts' }]
      }
    ],
    search: {
      provider: 'local'
    },
    socialLinks: [{ icon: 'github', link: 'https://github.com/jaeyoung0509/Openportio' }],
    editLink: {
      pattern: 'https://github.com/jaeyoung0509/Openportio/edit/develop/website/docs/:path',
      text: 'Edit this page on GitHub'
    },
    footer: {
      message: 'Built with VitePress',
      copyright: 'Copyright © Openportio contributors'
    }
  }
});
