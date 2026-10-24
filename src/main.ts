import './app/styles/theme.css';
import { installAcceptanceProbe } from './lib/markdown-document/acceptanceProbe';
import './app/styles/global.css';
import { mount } from 'svelte';
import { installAutoHideScrollbars } from './app/services/scrollbarVisibility';
import { bootstrapThemeFromSnapshot } from './app/services/themeManager';
import { createPerfTimer, initializeLogger, logError, logInfo } from './lib/services/logger';

bootstrapThemeFromSnapshot();
installAcceptanceProbe();

const target = document.getElementById('app');
const startupTimer = createPerfTimer('App', '前端入口加载');
initializeLogger();

if (!target) {
  logError('App', 'Nomo app root was not found.');
  throw new Error('Nomo app root was not found.');
}

const uninstallAutoHideScrollbars = installAutoHideScrollbars();
window.addEventListener('beforeunload', uninstallAutoHideScrollbars, { once: true });

const searchParams = new URLSearchParams(window.location.search);
const isSettingsView = searchParams.get('view') === 'settings';
logInfo('App', '开始加载根组件', { view: isSettingsView ? 'settings' : 'main' });
const rootComponentPromise = isSettingsView
  ? import('./app/components/SettingsWindow.svelte')
  : Promise.all([
      import('katex/dist/katex.min.css'),
      import('prosemirror-view/style/prosemirror.css'),
      import('./app/styles/app.css'),
    ]).then(() => import('./app/App.svelte'));

const app = rootComponentPromise
  .then(({ default: RootComponent }) =>
    mount(RootComponent, {
      target,
    }),
  )
  .then((mountedApp) => {
    startupTimer.end({ view: isSettingsView ? 'settings' : 'main' });
    logInfo('App', '根组件挂载完成', { view: isSettingsView ? 'settings' : 'main' });
    return mountedApp;
  })
  .catch((error) => {
    logError('App', 'Failed to mount Nomo app root.', {
      error: error instanceof Error ? error.message : String(error),
    });
    throw error;
  });

export default app;
