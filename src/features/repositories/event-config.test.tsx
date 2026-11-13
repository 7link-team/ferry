import { afterEach, expect, it } from 'vitest';
import { i18n } from '../../shared/i18n/i18n';
import { repositoryEventActor, repositoryEventForm } from './event-config';

afterEach(async()=>{await i18n.changeLanguage('en-US');});

it.each(['zh-CN','en-US','ms-MY'])('distinguishes absent polling actors from provider actors in %s', async locale=>{
  await i18n.changeLanguage(locale);
  const polling={actor:' ',detail:{source:'polling',commits:[{author:'Commit Author'}]}};
  const missing={actor:'',detail:{source:'webhook'}};
  const bare={actor:'',source:'polling'};
  expect(repositoryEventActor(bare)).not.toBe(repositoryEventActor(missing));
  expect(repositoryEventActor(polling)).toContain('Commit Author');
  expect(repositoryEventActor(polling)).not.toBe('Commit Author');
  expect(repositoryEventActor({actor:'',detail:{commits:[{author:'A',committer:'Newest Committer'},{committer:'Older'}]}})).toContain('Newest Committer');
  expect(repositoryEventActor({actor:'',detail:{commit:{author:'Tag Target'}}})).toContain('Tag Target');
  expect(repositoryEventActor({actor:'',head_commit:[{committer:'Flattened'}]})).toContain('Flattened');
  expect(repositoryEventActor({...polling,actor:'Actual Pusher'})).toBe('Actual Pusher');
  expect(repositoryEventActor({actor:null})).toBe(repositoryEventActor(missing));
  const fields=repositoryEventForm().sections.flatMap(section=>section.fields);
  const actor=fields.find(field=>field.key==='actor')!;
  expect(actor.when?.(polling)).toBe(true);
  expect(actor.display?.(polling)).toBe(repositoryEventActor(polling));
  const source=fields.find(field=>field.key==='source')!;
  expect(source.when?.(polling)).toBe(true);
  expect(source.display?.(polling)).not.toBe('polling');
});
