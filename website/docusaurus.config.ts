import rehypeShiki from '@shikijs/rehype';
import {themes as prismThemes} from 'prism-react-renderer';
import type {Config} from '@docusaurus/types';
import type * as Preset from '@docusaurus/preset-classic';

const config: Config = {
  title: 'Broski Docs',
  tagline: 'Deterministic build orchestration for modern teams',
  favicon: 'img/favicon.svg',

  future: {
    v4: true,
  },

  url: 'https://broski-docs.vercel.app',
  baseUrl: '/broski_docs/',

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
          editUrl: 'https://github.com/himudigonda/Broski/tree/main/website/',
          rehypePlugins: [[rehypeShiki, {themes: {light: 'ayu-light', dark: 'ayu-dark'}}]],
        },
        blog: false,
        pages: {
          rehypePlugins: [[rehypeShiki, {themes: {light: 'ayu-light', dark: 'ayu-dark'}}]],
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
    image: 'img/branding/broski_banner.png',
    colorMode: {
      defaultMode: 'light',
      respectPrefersColorScheme: true,
    },
    navbar: {
      title: 'Broski Docs',
      logo: {
        alt: 'Broski Logo',
        src: 'img/branding/broski_logo_base.png',
      },
      items: [
        {
          type: 'docSidebar',
          sidebarId: 'docsSidebar',
          position: 'left',
          label: 'Docs',
        },
        {
          href: 'https://github.com/himudigonda/Broski',
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
            {label: 'GitHub', href: 'https://github.com/himudigonda/Broski'},
            {label: 'Issues', href: 'https://github.com/himudigonda/Broski/issues'},
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
      copyright: `Copyright © ${new Date().getFullYear()} Himansh Mudigonda. Built with Docusaurus. This codebase was built with help from Codex-5.3-Extra-High. Gemini 3.1 Pro helped with the docs.`,
    },
    prism: {
      theme: prismThemes.gruvboxMaterialLight,
      darkTheme: prismThemes.gruvboxMaterialDark,
      additionalLanguages: ['bash', 'diff', 'json', 'toml', 'rust', 'yaml', 'makefile'],
    },
  } satisfies Preset.ThemeConfig,
};

export default config;
