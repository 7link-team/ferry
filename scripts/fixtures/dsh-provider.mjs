// Deterministic provider for real DSH process acceptance; never calls a vendor.
import { LlmAdapter } from '__LLM_MODULE_URL__';

export const name = 'runtime-test-provider';
export const inject = ['llm'];
export function apply(ctx) {
  class FixtureAdapter extends LlmAdapter {
    async listModels(provider) {
      return ['model-one', 'model-two'].map(id => ({ provider, id, name: id, inputModalities: ['text'] }));
    }
    async resolveModel(provider, model) {
      if (!['model-one', 'model-two'].includes(model)) throw Error('Unknown fixture model');
      return { provider, id: model, name: model, context: { contextWindow: 100000 }, inputModalities: ['text'] };
    }
    async *stream(options) {
      // DSH can append first-turn instruction messages after the user prompt.
      // Select the explicit test request, not those native bootstrap messages.
      const index = options.messages.findLastIndex(message => message.role === 'user'
        && message.content.some(block => block.type === 'text' && block.text.includes('DSH_TEST:')));
      const prompt = options.messages[index]?.content.filter(block => block.type === 'text').map(block => block.text).join('') || '';
      const marker = prompt.indexOf('DSH_TEST:');
      const test = marker < 0 ? { action: 'text' } : JSON.parse(prompt.slice(marker + 9));
      const results = options.messages.slice(index + 1).filter(message => message.role === 'tool');
      if (test.action === 'wait') await new Promise(resolve => setTimeout(resolve, 60000));
      if (test.action === 'fail') {
        yield { type: 'finish', reason: { kind: 'error', failure: { code: 'TEST_FAILURE', message: 'Expected fixture failure' } } };
        return;
      }
      if (['mcp', 'write'].includes(test.action) && !results.length) {
        const toolName = test.action === 'mcp' ? 'mcp__ferry__ferry_request' : 'write';
        if (!options.tools.some(tool => tool.name === toolName)) throw Error('Fixture tool missing: ' + toolName);
        const args = test.action === 'mcp' ? { method: test.method || 'GET', path: test.path, ...(test.body ? { body: test.body } : {}) }
          : { file_path: 'runtime-side-effect.txt', content: 'approved DSH fixture write' };
        const id = 'call-' + crypto.randomUUID();
        const block = { type: 'tool-call', id, name: toolName, arguments: JSON.stringify(args) };
        yield { type: 'block-start', index: 0, blockType: 'tool-call' };
        yield { type: 'tool-call-delta', index: 0, id, name: toolName, argumentsDelta: block.arguments };
        yield { type: 'block-end', index: 0, block };
        yield { type: 'finish', reason: { kind: 'tool-calls' } };
        return;
      }
      const text = `DSH_TEST_RESULT model=${options.model} users=${options.messages.filter(message => message.role === 'user').length} results=${JSON.stringify(results)}`;
      yield { type: 'block-start', index: 0, blockType: 'text' };
      yield { type: 'text-delta', index: 0, text };
      yield { type: 'block-end', index: 0, block: { type: 'text', text } };
      yield { type: 'finish', reason: { kind: 'stop' } };
    }
  }
  const registration = ctx.llm.registerAdapter(['runtime-test'], new FixtureAdapter());
  ctx.effect(() => registration);
}
