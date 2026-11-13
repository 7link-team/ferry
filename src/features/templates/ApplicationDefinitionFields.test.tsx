import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, expect, it } from 'vitest';
import { i18n } from '../../shared/i18n/i18n';
import { ApplicationDefinitionFields } from './ApplicationDefinitionFields';

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
let root: Root | undefined;
afterEach(async () => { if (root) await act(() => root!.unmount()); root = undefined; document.body.replaceChildren(); await i18n.changeLanguage('en-US'); });
it.each(['zh-CN', 'en-US', 'ms-MY'])('preserves hierarchy and every edge value without fake controls in %s', async locale => {
  await i18n.changeLanguage(locale);
  const host = document.createElement('div'); document.body.append(host); root = createRoot(host);
  const value = { apiVersion: 'ferry/blueprint-v1', kind: 'Blueprint', metadata: { name: 'cluster' }, allowSingleHost: false,
    customRoot: 'extension-value', drivers: { docker: { supported: true } },
    roles: [{ name: 'nats', identity: 'stable', instances: { min: 0, max: 9, default: 3, odd: true },
      ports: [{ name: 'client', container: 4222 }], service: { image: 'nats:test', command: ['--name', 'two words', 'line1\nline2'],
        health: { readiness: { httpGet: { path: '/ready', port: 8222 } } },
        resources: { limits: { cpu: '0.5', memory: '256Mi' } },
        files: [{ fromTemplate: 'nats.conf', path: '/etc/nats.conf' }], volumes: [{ name: 'data', path: '/data' }],
        labels: { identity: 'stable', kind: 'Blueprint', custom: '<script>alert(1)</script>' },
        values: { empty: '', zero: 0, missing: null, list: [], map: {} }, unknown: { preserved: 'raw-value' } } }], endpoints: [] };
  await act(() => root!.render(<ApplicationDefinitionFields value={value} />));
  const roles = host.querySelector('[data-blueprint-group="roles"]')!;
  for (const text of ['nats:test', '/ready', '8222', '256Mi', 'nats.conf', '/data', 'raw-value', 'stable', 'Blueprint', 'null', '[]', '{}', '<script>alert(1)</script>']) expect(roles.textContent).toContain(text);
  expect([...roles.querySelectorAll('ol li')].map(item => item.textContent)).toEqual(['--name', 'two words', 'line1\nline2']);
  expect([...roles.querySelectorAll('dd')].filter(item => item.textContent === '0')).toHaveLength(2);
  expect(host.querySelector('input,select,textarea,script')).toBeNull();
  expect(host.querySelector('[data-blueprint-group="other"]')?.textContent).toContain('customRoot');
  expect(host.querySelector('[data-blueprint-group="other"]')?.textContent).toContain('extension-value');
  expect(host.textContent).not.toMatch(/roles\[0\]|blueprintDetails\.|resource-domain:|wizard\./);
  expect(host.textContent).toContain(locale === 'zh-CN' ? '空字符串' : locale === 'ms-MY' ? 'Rentetan kosong' : 'Empty string');
  expect(host.querySelector('[data-blueprint-group="definition"]')?.textContent).toContain(locale === 'zh-CN' ? '否' : locale === 'ms-MY' ? 'Tidak' : 'No');
});

it('retains service fields, card frames and column layout when switching to readonly', async () => {
  const host=document.createElement('div');document.body.append(host);root=createRoot(host);
  const value={apiVersion:'ferry/v1',kind:'Application',metadata:{name:'example'},services:[{name:'web',image:'nginx:alpine',replicas:0}]};
  await act(()=>root!.render(<ApplicationDefinitionFields value={value} readOnly={false} />));
  const layout=()=>({
    cards:[...host.querySelectorAll('[data-blueprint-group], [data-blueprint-item]')].map(node=>node.className),
    grids:[...host.querySelectorAll('dl')].map(node=>node.className),
    labels:[...host.querySelectorAll('dt')].map(node=>node.textContent),
  });
  const editable=layout();
  await act(()=>root!.render(<ApplicationDefinitionFields value={value} readOnly />));
  expect(layout()).toEqual(editable);
  expect([...host.querySelectorAll('dd > [data-blueprint-value]')].every(node=>node.classList.contains('border'))).toBe(true);
  expect(host.querySelector('[data-blueprint-item="services"]')?.textContent).toContain('web');
  expect(host.querySelector('input, select, textarea, button')).toBeNull();
});
