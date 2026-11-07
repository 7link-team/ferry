import { api } from '../../shared/api/api';
import { Archive } from 'lucide-react';
import { deleteResource } from '../../shared/api/resource';
import { commonText, workflowText as t } from '../../shared/i18n/i18n';
import { ago, form, list, namedOptions, type ResourceConfigContext } from '../../shared/resources/config';
import { Badge } from '../../shared/ui/badge';
import { emptyGraph } from './graph';
import { WorkflowEditor, type WorkflowRow } from './WorkflowEditor';
import { templateGenerator } from '../templates/resource-config';

export function workflowsLists({ workspace }: ResourceConfigContext) {
  return {
    workflows: list<WorkflowRow>({
      resource: 'workflows', rowKey: row => row.id,
      filters: [
        { key: 'q', label: commonText('common.name'), type: 'text' },
        { key: 'group_id', label: commonText('table.groups'), type: 'select', options: namedOptions(workspace, 'workflow-groups') },
        { key: 'lifecycle', label: t('state'), type: 'select', options: [{ value: 'active', label: t('draft') + ' / ' + t('published') }, { value: 'archived', label: t('archived') }] },
      ],
      columns: [
        { key: 'name', label: commonText('common.name'), sortable: true },
        { key: 'lifecycle', label: t('state'), render: row => <Badge variant="outline">{row.lifecycle === 'archived' ? t('archived') : !row.published_revision_id ? t('draft') : row.published_revision_id === row.draft_revision_id ? t('published') : t('changes')}</Badge> },
        { key: 'description', label: commonText('common.description') },
        { key: 'updated_at', label: t('updatedAt'), labelKey: 'workflow-domain:updatedAt', sortable: true, render: row => ago(Number(row.updated_at)) },
      ],
      actions: [
        { label: commonText('common.view'), to: row => ({ id: row.id, mode: 'view' }) },
        { label: commonText('common.edit'), permission: 'workflows.update', when: row => row.lifecycle === 'active', to: row => ({ id: row.id, mode: 'edit' }) },
        { label: commonText('common.copy'), permission: 'workflows.create', to: () => ({ mode: 'new' }), query: row => ({ copy: row.id }) },
        { label: t('archive'), icon: <Archive className="size-4" />, permission: 'workflows.update', when: row => row.lifecycle === 'active', destructive: true, action: async row => { await api(`/workspaces/${workspace}/workflows/${row.id}`, { method: 'PATCH', body: JSON.stringify({ version: row.version, lifecycle: 'archived' }) }); } },
        { label: commonText('common.delete'), permission: 'workflows.delete', when: row => row.lifecycle === 'archived', destructive: true, action: row => deleteResource(workspace, 'workflows', row.id) },
      ],
    }),
    'workflow-groups': list<{ id: string; name: string }>({ resource: 'workflow-groups', rowKey: row => row.id, columns: [{ key: 'name', label: commonText('common.name'), sortable: true }], actions: [{ label: commonText('common.edit'), permission: 'workflow-groups.update', to: row => ({ id: row.id, mode: 'edit' }) }, { label: commonText('common.delete'), permission: 'workflow-groups.delete', destructive: true, action: row => deleteResource(workspace, 'workflow-groups', row.id) }] }),
  };
}
export function workflowsForms({ workspace, permissions, renderTable }: ResourceConfigContext) {
  return {
    workflows: form<WorkflowRow>({
      resource: 'workflows', width: 'full', editor: { field: 'definition', get nameLabel() { return t('name'); } },
      ...(permissions?.has('pipeline-templates.read') ? {generator:templateGenerator(workspace,'pipeline-templates','definition','nodes')} : {}),
      heading: row => row.name, blank: { name: '', description: '', group_id: '0', definition: JSON.stringify(emptyGraph()), lifecycle: 'active' },
      copy: row => ({ name: row.name + ' (copy)', description: row.description, group_id: row.group_id, definition: row.definition, lifecycle: 'active' }),
      readOnly: row => row.lifecycle === 'archived',
      sections: [{ fields: [
        { key: 'name', label: t('name'), type: 'text', required: true },
        { key: 'group_id', label: commonText('table.groups'), type: 'select', options: namedOptions(workspace, 'workflow-groups'), emptyValues: ['0'] },
        { key: 'description', label: commonText('common.description'), type: 'textarea' },
        { key: 'definition', label: t('canvas'), type: 'custom', span: 2, render: props => <WorkflowEditor {...props} permissions={permissions ?? new Set()} {...(renderTable ? { renderTable } : {})} /> },
      ] }],
    }),
    'workflow-groups': form<{ id: string; name: string; description: string }>({ resource: 'workflow-groups', heading: row => row.name, blank: { name: '', description: '' }, sections: [{ fields: [{ key: 'name', label: commonText('common.name'), type: 'text', required: true }, { key: 'description', label: commonText('common.description'), type: 'textarea' }] }] }),
  };
}
