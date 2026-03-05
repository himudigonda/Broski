import type {SidebarsConfig} from '@docusaurus/plugin-content-docs';

const sidebars: SidebarsConfig = {
  docsSidebar: [
    'index',
    {
      type: 'category',
      label: 'Quickstart',
      items: ['quickstart/install', 'quickstart/first-pleasefile', 'quickstart/make-just-please'],
    },
    {
      type: 'category',
      label: 'Architecture',
      items: ['architecture/engine-overview', 'architecture/cache-explain'],
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
        'operations/release-runbook',
      ],
    },
    {
      type: 'category',
      label: 'Release Notes',
      items: ['releases/v0.5', 'releases/v0.4'],
    },
  ],
};

export default sidebars;
