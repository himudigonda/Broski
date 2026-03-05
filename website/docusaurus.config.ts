import rehypeShiki from '@shikijs/rehype';
import {themes as prismThemes} from 'prism-react-renderer';
import type {Config} from '@docusaurus/types';
import type * as Preset from '@docusaurus/preset-classic';

const config: Config = {
  title: 'Please Docs',
  tagline: 'Deterministic build orchestration for modern teams',
  favicon: 'img/favicon.svg',

  future: {
    v4: true,
  },

  url: 'https://please-docs.vercel.app',
  baseUrl: '/please_docs/',

  onBrokenLinks: 'throw',
  i18n: {
    defaultLocale: 'en',
    locales: ['en'],
  },

  markdown: {
    mermaid: true,
    hooks: {
      onBrokenMarkdownLinks: 'throw',
    },
  },

  themes: ['@docusaurus/theme-mermaid'],

  presets: [
    [
      'classic',
      {
        docs: {
          routeBasePath: '/',
          sidebarPath: './sidebars.ts',
          editUrl: 'https://github.com/himudigonda/Please/tree/main/website/',
          rehypePlugins: [[rehypeShiki, {themes: {light: 'github-light', dark: 'github-dark'}}]],
          lastVersion: 'current',
          versions: {
            current: {
              label: 'v0.5',
            },
            'v0.4': {
              label: 'v0.4 (archived)',
              path: 'v0.4',
            },
          },
        },
        blog: false,
        pages: {
          rehypePlugins: [[rehypeShiki, {themes: {light: 'github-light', dark: 'github-dark'}}]],
        },
        theme: {
          customCss: './src/css/custom.css',
        },
      } satisfies Preset.Options,
    ],
  ],

  plugins: [
    [
      'docusaurus-plugin-search-local',
      {
        hashed: true,
        docsRouteBasePath: '/',
        docsDir: 'docs',
        indexDocs: true,
        indexPages: true,
      },
    ],
  ],

  themeConfig: {
    image: 'img/og-eclipse.png',
    colorMode: {
      defaultMode: 'dark',
      respectPrefersColorScheme: true,
    },
    navbar: {
      title: 'Please Eclipse Portal',
      logo: {
        alt: 'Please Logo',
        src: 'img/logo.svg',
      },
      items: [
        {
          type: 'docSidebar',
          sidebarId: 'docsSidebar',
          position: 'left',
          label: 'Docs',
        },
        {
          type: 'docsVersionDropdown',
          position: 'left',
          dropdownItemsAfter: [
            {
              to: 'https://github.com/himudigonda/Please/tree/main/docs/legacy',
              label: 'Legacy Markdown Archive',
            },
          ],
        },
        {
          href: 'https://github.com/himudigonda/Please',
          label: 'GitHub',
          position: 'right',
        },
        {
          href: 'https://himudigonda.me',
          label: 'himudigonda.me',
          position: 'right',
        },
      ],
    },
    footer: {
      style: 'dark',
      links: [
        {
          title: 'Product',
          items: [
            {label: 'Quickstart', to: '/'},
            {label: 'DSL Reference', to: '/dsl/overview'},
            {label: 'Migration', to: '/operations/migration'},
          ],
        },
        {
          title: 'Community',
          items: [
            {label: 'GitHub', href: 'https://github.com/himudigonda/Please'},
            {label: 'Issues', href: 'https://github.com/himudigonda/Please/issues'},
          ],
        },
        {
          title: 'Ecosystem',
          items: [
            {label: 'Portfolio', href: 'https://himudigonda.me'},
            {label: 'Install Guide', to: '/quickstart/install'},
          ],
        },
      ],
      copyright: `Copyright © ${new Date().getFullYear()} Himansh Mudigonda. Built with Docusaurus.`,
    },
    prism: {
      theme: prismThemes.github,
      darkTheme: prismThemes.dracula,
      additionalLanguages: ['bash', 'diff', 'json', 'toml', 'rust', 'yaml'],
    },
  } satisfies Preset.ThemeConfig,
};

export default config;
