declare module 'markdown-it/lib/rules_block/table.mjs' {
  import type StateBlock from 'markdown-it/lib/rules_block/state_block.mjs';
  const table: (state: StateBlock, startLine: number, endLine: number, silent: boolean) => boolean;
  export default table;
}
