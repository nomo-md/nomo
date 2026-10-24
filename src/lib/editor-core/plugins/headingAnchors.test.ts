import { expect, it } from 'vitest';
import { EditorState } from 'prosemirror-state';
import { schema } from '../schema';
import { headingAnchorsKey, headingAnchorsPlugin } from './headingAnchors';

it('事务映射保留临时标题身份且不分配 URL id，原生校准后替换', () => {
  const heading = schema.nodes.heading.create({level:1},schema.text('A'));
  let state = EditorState.create({doc:schema.nodes.doc.create(null,[heading]),plugins:[headingAnchorsPlugin()]});
  state=state.apply(state.tr.setMeta(headingAnchorsKey,['a']));
  state=state.apply(state.tr.insert(state.doc.content.size,schema.nodes.heading.create({level:2},schema.text('B'))));
  const added=headingAnchorsKey.getState(state)!.decorations.find().find(d=>d.spec.local)!;
  expect(added.spec.headingId).toMatch(/^local-heading:/);
  const id=added.spec.headingId;
  state=state.apply(state.tr.insertText('new',added.from+1));
  expect(headingAnchorsKey.getState(state)!.decorations.find().some(d=>d.spec.headingId===id)).toBe(true);
  expect(headingAnchorsKey.getState(state)!.ownedIds).toEqual(['a']);
  state=state.apply(state.tr.setMeta(headingAnchorsKey,['a','b']));
  expect(headingAnchorsKey.getState(state)!.decorations.find().map(d=>d.spec.headingId)).toEqual(['a','b']);
});
