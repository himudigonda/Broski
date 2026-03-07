import type {SidebarsConfig} from '@docusaurus/plugin-content-docs';

const sidebars: SidebarsConfig = {
  docsSidebar: [
    'index',
    {
      type: 'category',
      label: 'Quickstart',
      items: [
        'quickstart/thirty-second-quickstart',
        'quickstart/install',
        'quickstart/first-broskifile',
        'quickstart/make-just-broski',
      ],
    },
    {
      type: 'category',
      label: 'Architecture',
      items: ['architecture/why-broski', 'architecture/engine-overview', 'architecture/cache-explain'],
    },
    {
      type: 'category',
      label: 'DSL Reference',
      items: [
        'dsl/overview',
        'dsl/tasks-and-params',
        'dsl/annotations',
        'dsl/interpolation-and-builtins',
        'dsl/imports-and-decorators',
        'dsl/reference-table',
      ],
    },
    {
      type: 'category',
      label: 'CLI + Operations',
      items: [
        'cli/commands',
        'cli/watch-mode',
        'operations/security',
        'operations/migration',
        'operations/anti-patterns',
        'operations/common-mistakes',
        'operations/faq',
        'operations/troubleshooting',
        'operations/release-runbook',
      ],
    },
    {
      type: 'category',
      label: 'Release Notes',
      items: ['releases/v0.5'],
    },
  ],
};

export default sidebars;
