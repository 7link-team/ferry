import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { parse } from "yaml";
import { ApplicationTemplateEditor } from "./ApplicationTemplateEditor";
import { readManifest } from "../deployments/manifest/model";
import { commonText, hostRepositoryText, i18n } from "../../shared/i18n/i18n";
import type { TemplateParameter } from "./TemplateParametersEditor";
import { named } from "../../shared/api/resource";

vi.mock("../../shared/editor/CodeEditor", () => ({ CodeEditor: ({ value, readOnly, onChange }: { value: string; readOnly: boolean; onChange: (value: string) => void }) => <textarea aria-label="source" value={value} readOnly={readOnly} onChange={event => onChange(event.target.value)} /> }));
const fixture = `# original application
apiVersion: ferry/v1
kind: Application
metadata: {name: web, labels: {release: "01"}}
services:
  - name: web
    image: nginx:alpine # keep image comment
    replicas: 1
    env: [{name: TOKEN, valueFrom: variable/token}]
    dependsOn: [db]
    resources: null
    health: null
  - name: db
    image: postgres:17
    command: [postgres, -c, shared_buffers=256MB]
overrides: {compose: {services: {web: {restart: unless-stopped}}}}
`;
let root: Root, host: HTMLDivElement, latest: string, blocked: boolean;
function Harness({ source = fixture, readOnly = false, mode = "compose", workspace = "", parameters }: { source?: string; readOnly?: boolean; mode?: string; workspace?: string; parameters?: TemplateParameter[] }) {
  const [value, setValue] = useState(source);latest = value;
  return <ApplicationTemplateEditor value={value} mode={mode} readOnly={readOnly} workspace={workspace} {...(parameters ? { templateParameters: parameters } : {})}
    onChange={setValue} onValidityChange={value => { blocked = value; }} />;
}
const field = (path: (string | number)[]) => [...host.querySelectorAll<HTMLElement>('[data-definition-path]')].find(node => node.dataset.definitionPath === JSON.stringify(path))!;
const control = (path: (string | number)[]) => field(path).querySelector<HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement>('input,textarea,select')!;
const button = (name: string) => [...host.querySelectorAll('button')].find(node => node.textContent === name)!;
async function fill(element: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement, value: string) {
  await act(() => {
    const prototype = element instanceof HTMLSelectElement ? HTMLSelectElement.prototype : element instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, 'value')!.set!.call(element, value);
    element.dispatchEvent(new Event(element instanceof HTMLSelectElement ? 'change' : 'input', { bubbles: true }));
  });
}
async function click(name: string) { await act(async () => button(name).click()); }
beforeEach(async () => {
  await i18n.changeLanguage('zh-CN');Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  host = document.createElement('div');document.body.append(host);root = createRoot(host);blocked = false;
});
afterEach(async () => { await act(() => root.unmount());host.remove();vi.unstubAllGlobals();await i18n.changeLanguage('en-US'); });

it('preserves comments, typed labels, secrets, command arrays and overrides when editing and toggling source', async () => {
  await act(() => root.render(<Harness />));await fill(control(['services', 0, 'image']), 'nginx:stable');
  const expected = parse(fixture);expected.services[0].image = 'nginx:stable';expect(parse(latest)).toEqual(expected);
  expect(latest).toContain('# keep image comment');expect(latest).toContain('# original application');
  await click(hostRepositoryText('manifest.019'));const source = latest;await click(hostRepositoryText('manifest.018'));expect(latest).toBe(source);
});
it('renames service references atomically, rejects duplicate names and blocks deleting referenced services', async () => {
  await act(() => root.render(<Harness />));await fill(control(['services', 1, 'name']), 'web');expect(parse(latest).services[1].name).toBe('db');
  await fill(control(['services', 1, 'name']), 'database');expect(parse(latest).services[0].dependsOn).toEqual(['database']);
  expect(parse(latest).services[1].command).toEqual(['postgres', '-c', 'shared_buffers=256MB']);
  const service = host.querySelectorAll('[data-blueprint-item="services"]')[1]!;
  await act(() => service.querySelector<HTMLButtonElement>('button')!.click());
  expect(document.querySelector('[role="dialog"]')).toBeNull();expect(latest).toContain('database');expect(host.textContent).toContain(hostRepositoryText('manifest.028'));
});
it('updates Blueprint role colocation and endpoint references on rename', async () => {
  const source = JSON.stringify({ apiVersion: 'ferry/blueprint-v1', kind: 'Blueprint', metadata: { name: 'db' }, roles: [{ name: 'db' }, { name: 'proxy', placement: { colocateWith: 'db' } }], endpoints: [{ name: 'client', role: 'db', port: 'client' }] });
  await act(() => root.render(<Harness source={source} />));await fill(control(['roles', 0, 'name']), 'database');
  expect(parse(latest).roles[1].placement.colocateWith).toBe('database');expect(parse(latest).endpoints[0].role).toBe('database');
});
it('blocks malformed source, retains the draft and recovers after repair', async () => {
  await act(() => root.render(<Harness />));await click(hostRepositoryText('manifest.019'));
  await fill(host.querySelector('textarea[aria-label="source"]')!, 'services: [');expect(blocked).toBe(true);expect(latest).toBe('services: [');
  expect(button(hostRepositoryText('manifest.018')).disabled).toBe(true);
  await fill(host.querySelector('textarea[aria-label="source"]')!, fixture);expect(blocked).toBe(false);
  await click(hostRepositoryText('manifest.018'));expect(control(['services', 0, 'image']).value).toBe('nginx:alpine');
});
it('creates optional objects from null and adds nested typed fields without an advanced JSON editor', async () => {
  await act(() => root.render(<Harness />));
  await act(() => field(['services', 0, 'resources']).querySelector<HTMLButtonElement>('button')!.click());
  const add = [...host.querySelectorAll<HTMLElement>('[data-definition-add]')].find(node => node.dataset.definitionAdd === JSON.stringify(['services', 0, 'resources']))!;
  await fill(add.querySelector('select')!, 'limits');await act(() => add.querySelector<HTMLButtonElement>('button')!.click());
  const limits = [...host.querySelectorAll<HTMLElement>('[data-definition-add]')].find(node => node.dataset.definitionAdd === JSON.stringify(['services', 0, 'resources', 'limits']))!;
  await fill(limits.querySelector('select')!, 'cpu');await act(() => limits.querySelector<HTMLButtonElement>('button')!.click());
  await fill(control(['services', 0, 'resources', 'limits', 'cpu']), '500m');expect(parse(latest).services[0].resources).toEqual({ limits: { cpu: '500m' } });
  expect(parse(latest).services[0].health).toBeNull();expect(blocked).toBe(false);
});
it('adds native artifact services and readonly cannot mutate the configuration', async () => {
  await act(() => root.render(<Harness mode="native" />));
  const group = host.querySelector('[data-blueprint-group="services"]')!;
  const add = [...group.querySelectorAll('button')].find(node => node.textContent === hostRepositoryText('manifest-extra.003', { value1: commonText('resource-domain:deployment.servicesSection') }))!;
  await act(() => add.click());expect(parse(latest).services[2]).toEqual({ name: 'service-3', artifact: { source: 'pipeline', pipeline: '', path: '' } });
  const expected = latest;await act(() => root.render(<Harness mode="native" readOnly />));expect(host.querySelector('input,select,textarea')).toBeNull();expect(host.querySelectorAll('button')).toHaveLength(1);expect(latest).toBe(expected);
});
it.each(['docker', 'compose', 'stack', 'native'])('restricts %s source selection while preserving incompatible stored source until explicitly switched', async mode => {
  const source = mode === 'native' ? fixture : fixture.replace('image: nginx:alpine # keep image comment', 'artifact: {source: pipeline, pipeline: build, path: bin/web}');
  await act(() => root.render(<Harness mode={mode} source={source} />));const picker = host.querySelector<HTMLSelectElement>(`select[aria-label="${hostRepositoryText('manifest.033')}"]`)!;
  expect([...picker.options].filter(option => !option.disabled).map(option => option.value)).toEqual([mode === 'native' ? 'artifact' : 'image']);expect(picker.value).toBe('');expect(latest).toBe(source);
  await fill(picker, mode === 'native' ? 'artifact' : 'image');
  const service = parse(latest).services[0];expect(mode === 'native' ? service.image : service.artifact).toBeUndefined();expect(mode === 'native' ? service.artifact : service.image).toEqual(mode === 'native' ? { source: 'pipeline', pipeline: '', path: '' } : '');
});
it('preserves dual sources and unsupported placement across engines and blocks destructive downgrade', async () => {
  const source = fixture.replace('ferry/v1', 'ferry/v2').replace('replicas: 1', 'artifact: {source: pipeline, pipeline: build, path: bin/web}\n    placement: {constraints: [node.role==manager], maxReplicasPerNode: 2}\n    replicas: 1');
  await act(() => root.render(<Harness source={source} />));
  for (const mode of ['docker', 'compose', 'stack', 'native']) {
    await act(() => root.render(<Harness mode={mode} />));expect(latest).toBe(source);
    expect(host.querySelector<HTMLSelectElement>(`select[aria-label="${hostRepositoryText('manifest.033')}"]`)!.value).toBe('both');
  }
  await fill(control(['apiVersion']), 'ferry/v1');expect(latest).toBe(source);expect(host.textContent).toContain(commonText('resource-domain:deployment.downgradeBlocked'));
  await fill(control(['services', 0, 'placement', 'maxReplicasPerNode']), '3');expect(parse(latest).services[0].placement).toEqual({ constraints: ['node.role==manager'], maxReplicasPerNode: 3 });
});
it('uses current unsaved configuration for capability checks and clears obsolete results', async () => {
  const fetch = vi.fn(async (_url: string, _options?: RequestInit) => ({ ok: true, status: 200, json: async () => ({ checks: [{ mode: 'compose', supported: true }, { mode: 'native', supported: false, reason: 'Native requires artifact' }] }) }));vi.stubGlobal('fetch', fetch);
  await act(() => root.render(<Harness workspace="ops" />));await click(commonText('resource-domain:deployment.checkCompatibility'));
  expect(fetch.mock.calls[0]?.[0]).toBe('/api/workspaces/ops/deployment-capabilities');expect(JSON.parse(String(fetch.mock.calls[0]?.[1]?.body)).manifest).toBe(fixture);expect(host.textContent).toContain('Native requires artifact');
  await fill(control(['services', 0, 'image']), 'nginx:stable');expect(host.textContent).not.toContain('Native requires artifact');
});
it('shows empty or unavailable pipeline/build references without silently clearing saved IDs', async () => {
  const fetch = vi.fn(async () => ({ ok: true, status: 200, json: async () => ({ items: [], total: 0, page_size: 200 }) }));vi.stubGlobal('fetch', fetch);
  const source = fixture.replace('image: nginx:alpine # keep image comment', 'artifact: {source: pipeline, pipeline: missing, run: saved-run, path: dist}');
  await act(async () => root.render(<Harness source={source} workspace="ops" mode="native" />));
  expect(field(['services', 0, 'artifact', 'pipeline']).textContent).toContain(named(undefined, 'missing'));
  expect(field(['services', 0, 'artifact', 'run']).textContent).toContain(named(undefined, 'saved-run'));expect(latest).toBe(source);
});
it('shows successful builds with no completion timestamp without inventing a date', async () => {
  vi.stubGlobal('fetch', vi.fn(async (url: string) => ({ ok: true, status: 200, json: async () => ({ items: url.includes('pipeline-runs') ? [{ id: 'r1', number: 1, finished_at: 0 }] : [{ id: 'p1', name: 'Build' }], total: 1, page_size: 200 }) })));
  const source = fixture.replace('image: nginx:alpine # keep image comment', 'artifact: {source: pipeline, pipeline: p1, run: r1, path: dist}');
  await act(async () => root.render(<Harness source={source} workspace="ops" mode="native" />));
  expect(field(['services', 0, 'artifact', 'run']).textContent).toContain('#1 · —');expect(latest).toBe(source);
});
it('updates pipeline and environment sources atomically without losing unrelated comments', async () => {
  const source = fixture.replace('image: nginx:alpine # keep image comment', 'artifact: {source: pipeline, pipeline: old, run: old-run, path: dist} # keep artifact');
  await act(() => root.render(<Harness source={source} mode="native" />));await fill(control(['services', 0, 'artifact', 'pipeline']), 'new');
  expect(parse(latest).services[0].artifact).toEqual({ source: 'pipeline', pipeline: 'new', path: 'dist' });expect(latest).toContain('# keep artifact');
  const add = [...host.querySelectorAll<HTMLElement>('[data-definition-add]')].find(node => node.dataset.definitionAdd === JSON.stringify(['services', 0, 'env', 0]))!;
  await fill(add.querySelector('select')!, 'value');await act(() => add.querySelector<HTMLButtonElement>('button')!.click());
  expect(parse(latest).services[0].env[0]).toEqual({ name: 'TOKEN', value: '' });
});
it('binds numeric/string/secret fields by type and restores numeric YAML literals after an expression', async () => {
  const parameters: TemplateParameter[] = [{ name: 'image', type: 'string' }, { name: 'replicas', type: 'integer' }, { name: 'token', type: 'string', secret: true }];const fetch = vi.fn();vi.stubGlobal('fetch', fetch);
  await act(() => root.render(<Harness mode="" workspace="ops" parameters={parameters} />));
  expect(field(['kind']).querySelector('select')).toBeNull();expect(field(['apiVersion']).querySelectorAll('select')).toHaveLength(1);
  for (const [path, name] of [[['services', 0, 'image'], 'image'], [['services', 0, 'replicas'], 'replicas'], [['services', 0, 'env', 0, 'valueFrom'], 'token']] as const) {
    const picker = field([...path]).querySelector<HTMLSelectElement>('select[aria-label]')!;
    expect([...picker.options].map(option => option.value).filter(Boolean)).toEqual([name]);await fill(picker, name);
  }
  expect(parse(latest).services[0]).toMatchObject({ image: '{{ .Values.image }}', replicas: '{{ .Values.replicas }}', env: [{ valueFrom: '{{ .Values.token }}' }] });
  const picker = field(['services', 0, 'replicas']).querySelector<HTMLSelectElement>('select')!;expect([...picker.options].map(option => option.value).filter(Boolean)).toEqual(['replicas']);
  await fill(control(['services', 0, 'replicas']), '4');expect(parse(latest).services[0].replicas).toBe(4);expect(fetch).not.toHaveBeenCalled();expect(blocked).toBe(false);
});
it('edits Blueprint root booleans and unknown nested objects without locking or flattening them', async () => {
  await act(() => root.render(<Harness source={JSON.stringify({ apiVersion: 'ferry/blueprint-v1', metadata: { name: 'db' }, allowSingleHost: false, roles: [], extension: { nested: { flag: true } } })} />));
  await fill(control(['allowSingleHost']), 'true');await fill(control(['extension', 'nested', 'flag']), 'false');
  expect(parse(latest)).toMatchObject({ allowSingleHost: true, extension: { nested: { flag: false } } });
});
it('keeps source and values unchanged when switching the six existing UI languages', async () => {
  await act(() => root.render(<Harness />));const expected = latest;
  for (const locale of ['en-US', 'zh-CN', 'zh-TW', 'ja-JP', 'ko-KR', 'ms-MY']) {
    await act(async () => { await i18n.changeLanguage(locale); });expect(host.textContent).toContain(hostRepositoryText('manifest.032'));expect(latest).toBe(expected);
  }
});
it('retains model rejection of duplicate keys, aliases and malformed collection shapes', () => {
  expect(() => readManifest(fixture + '\nservices: []')).toThrow();expect(() => readManifest('metadata: &m {name: test}\nservices: [*m]')).toThrow();
  expect(() => readManifest('metadata: {name: test}\nservices: [{name: test, env: wrong}]')).toThrow('env');expect(() => readManifest('metadata: {name: test}\nservices: [wrong]')).toThrow('services');
});
