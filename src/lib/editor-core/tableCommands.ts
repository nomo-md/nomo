import type { Node as ProseMirrorNode } from 'prosemirror-model';
import { TextSelection, type Command, type EditorState, type Transaction } from 'prosemirror-state';
import {
  addColumnAfter,
  addColumnBefore,
  addRowAfter,
  addRowBefore,
  CellSelection,
  deleteColumn,
  deleteRow,
  deleteTable,
  selectedRect,
  TableMap,
} from 'prosemirror-tables';
import { schema, type TableColumnAlignment } from './schema';

interface TableContext {
  table: ProseMirrorNode;
  tableStart: number;
  rowIndex: number;
  columnIndex: number;
  map: TableMap;
}

export function createTableNode(bodyRows: number, columns: number): ProseMirrorNode {
  const columnCount = Math.max(2, Math.min(columns, 8));
  const bodyRowCount = Math.max(1, Math.min(bodyRows, 12));
  const rows: ProseMirrorNode[] = [];

  rows.push(createTableRow(columnCount, 0, true));
  for (let rowIndex = 1; rowIndex <= bodyRowCount; rowIndex += 1) {
    rows.push(createTableRow(columnCount, rowIndex, false));
  }

  return schema.nodes.table.createChecked(null, rows);
}

export function addTableRowBefore(): Command {
  return addRowBefore;
}

export function addTableRowAfter(): Command {
  return (state, dispatch) => {
    const context = findTableContext(state);
    if (!context) return false;

    return addRowAfter(state, (tr) => {
      moveSelectionToInsertedRow(tr, context);
      dispatch?.(tr.scrollIntoView());
    });
  };
}

export function addTableColumnBefore(): Command {
  return addColumnBefore;
}

export function addTableColumnAfter(): Command {
  return addColumnAfter;
}

export function deleteCurrentTableRow(): Command {
  return deleteRow;
}

export function deleteCurrentTableColumn(): Command {
  return deleteColumn;
}

export function deleteCurrentTable(): Command {
  return deleteTable;
}

export function resizeCurrentTable(rows: number, columns: number): Command {
  return (state, dispatch) => {
    const context = findTableContext(state);
    if (!context) return false;

    const targetRows = clampTableSize(rows, 1, 20);
    const targetColumns = clampTableSize(columns, 1, 12);
    const resizedTable = resizeTableNode(context.table, targetRows, targetColumns);
    if (resizedTable.eq(context.table)) return false;

    const tablePosition = context.tableStart - 1;
    const tr = state.tr.replaceWith(
      tablePosition,
      tablePosition + context.table.nodeSize,
      resizedTable,
    );
    moveSelectionToResizedTable(
      tr,
      context.tableStart,
      resizedTable,
      Math.min(context.rowIndex, targetRows - 1),
      Math.min(context.columnIndex, targetColumns - 1),
    );
    dispatch?.(tr.scrollIntoView());
    return true;
  };
}

export function setTableColumnAlignment(align: TableColumnAlignment): Command {
  return (state, dispatch) => {
    const context = findTableContext(state);
    if (!context) return false;

    const columns =
      state.selection instanceof CellSelection
        ? selectedRect(state)
        : { left: context.columnIndex, right: context.columnIndex + 1 };
    const tr = state.tr;
    const touched = new Set<number>();

    // Markdown 对齐属于整列；部分行的单元格选区也需要更新对应列的所有行。
    for (let rowIndex = 0; rowIndex < context.map.height; rowIndex += 1) {
      for (let columnIndex = columns.left; columnIndex < columns.right; columnIndex += 1) {
        const cellPosition =
          context.tableStart + context.map.map[rowIndex * context.map.width + columnIndex];
        if (touched.has(cellPosition)) continue;
        touched.add(cellPosition);

        const cell = state.doc.nodeAt(cellPosition);
        if (cell && cell.attrs.align !== align) {
          tr.setNodeMarkup(cellPosition, undefined, { ...cell.attrs, align });
        }
      }
    }
    dispatch?.(tr.scrollIntoView());
    return true;
  };
}

export function toggleFirstTableRowHeader(): Command {
  return (state, dispatch) => {
    const context = findTableContext(state);
    if (!context || context.map.height === 0) return false;

    const firstRowCells = Array.from({ length: context.map.width }, (_, columnIndex) => {
      const position = context.tableStart + context.map.positionAt(0, columnIndex, context.table);
      return { position, node: state.doc.nodeAt(position) };
    }).filter((cell): cell is { position: number; node: ProseMirrorNode } => Boolean(cell.node));

    const shouldDisableHeader = firstRowCells.every(
      (cell) => cell.node.type === schema.nodes.table_header,
    );
    const targetType = shouldDisableHeader ? schema.nodes.table_cell : schema.nodes.table_header;
    const tr = state.tr;
    for (const cell of firstRowCells) {
      tr.setNodeMarkup(cell.position, targetType, cell.node.attrs);
    }
    dispatch?.(tr.scrollIntoView());
    return true;
  };
}

export function normalizeFirstTableRowHeader(): Command {
  return (state, dispatch) => {
    const context = findTableContext(state);
    if (!context || context.map.height === 0) return false;

    const tr = state.tr;
    const touched = new Set<number>();

    for (let rowIndex = 0; rowIndex < context.map.height; rowIndex += 1) {
      const targetType = rowIndex === 0 ? schema.nodes.table_header : schema.nodes.table_cell;

      for (let columnIndex = 0; columnIndex < context.map.width; columnIndex += 1) {
        const cellPosition =
          context.tableStart + context.map.positionAt(rowIndex, columnIndex, context.table);
        if (touched.has(cellPosition)) continue;
        touched.add(cellPosition);

        const cell = state.doc.nodeAt(cellPosition);
        if (cell && cell.type !== targetType) {
          tr.setNodeMarkup(cellPosition, targetType, cell.attrs);
        }
      }
    }

    if (!tr.docChanged) return false;
    dispatch?.(tr.scrollIntoView());
    return true;
  };
}

function createTableRow(columns: number, rowIndex: number, header: boolean): ProseMirrorNode {
  const cellType = header ? schema.nodes.table_header : schema.nodes.table_cell;
  const cells = Array.from({ length: columns }, (_, columnIndex) => {
    return cellType.createAndFill(null, schema.nodes.paragraph.createAndFill());
  }).filter((cell): cell is ProseMirrorNode => Boolean(cell));

  return schema.nodes.table_row.createChecked(null, cells);
}

function resizeTableNode(
  table: ProseMirrorNode,
  targetRows: number,
  targetColumns: number,
): ProseMirrorNode {
  const rows: ProseMirrorNode[] = [];

  for (let rowIndex = 0; rowIndex < targetRows; rowIndex += 1) {
    const sourceRow = rowIndex < table.childCount ? table.child(rowIndex) : null;
    const headerRow = sourceRow ? isHeaderRow(sourceRow) : false;
    const cells: ProseMirrorNode[] = [];

    for (let columnIndex = 0; columnIndex < targetColumns; columnIndex += 1) {
      const sourceCell =
        sourceRow && columnIndex < sourceRow.childCount ? sourceRow.child(columnIndex) : null;
      cells.push(sourceCell ?? createEmptyTableCell(headerRow));
    }

    rows.push(schema.nodes.table_row.createChecked(sourceRow?.attrs ?? null, cells));
  }

  return schema.nodes.table.createChecked(table.attrs, rows);
}

function createEmptyTableCell(header: boolean): ProseMirrorNode {
  const cellType = header ? schema.nodes.table_header : schema.nodes.table_cell;
  const cell = cellType.createAndFill(null, schema.nodes.paragraph.createAndFill());
  if (!cell) {
    throw new Error('Unable to create an empty table cell.');
  }
  return cell;
}

function isHeaderRow(row: ProseMirrorNode): boolean {
  if (row.childCount === 0) return false;
  for (let index = 0; index < row.childCount; index += 1) {
    if (row.child(index).type !== schema.nodes.table_header) return false;
  }
  return true;
}

function clampTableSize(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) return min;
  return Math.max(min, Math.min(max, Math.floor(value)));
}

export function findTableContext(state: EditorState): TableContext | null {
  const { $from } = state.selection;
  let tableDepth = -1;
  let cellDepth = -1;

  for (let depth = $from.depth; depth > 0; depth -= 1) {
    const role = $from.node(depth).type.spec.tableRole;
    if (role === 'cell' || role === 'header_cell') {
      cellDepth = depth;
    }
    if (role === 'table') {
      tableDepth = depth;
      break;
    }
  }

  if (tableDepth < 0 || cellDepth < 0) return null;

  const table = $from.node(tableDepth);
  const tableStart = $from.before(tableDepth) + 1;
  const cellPosition = $from.before(cellDepth);
  const map = TableMap.get(table);
  const rect = map.findCell(cellPosition - tableStart);

  return {
    table,
    tableStart,
    rowIndex: rect.top,
    columnIndex: rect.left,
    map,
  };
}

function moveSelectionToInsertedRow(tr: Transaction, context: TableContext): void {
  const table = tr.doc.nodeAt(context.tableStart - 1);
  if (!table || table.type !== schema.nodes.table) return;

  const map = TableMap.get(table);
  const targetRow = Math.min(context.rowIndex + 1, map.height - 1);
  const targetColumn = Math.min(context.columnIndex, map.width - 1);
  const cellPos = context.tableStart + map.positionAt(targetRow, targetColumn, table);
  const cell = tr.doc.nodeAt(cellPos);
  if (!cell) return;

  let textPos: number | null = null;
  cell.descendants((node, pos) => {
    if (textPos === null && node.isTextblock) {
      textPos = cellPos + 1 + pos + 1;
      return false;
    }
    return true;
  });

  if (textPos !== null) {
    tr.setSelection(TextSelection.create(tr.doc, textPos));
  }
}

function moveSelectionToResizedTable(
  tr: Transaction,
  tableStart: number,
  table: ProseMirrorNode,
  row: number,
  column: number,
): void {
  const map = TableMap.get(table);
  const cellPos = tableStart + map.positionAt(row, column, table);
  const cell = tr.doc.nodeAt(cellPos);
  if (!cell) return;

  let textPos: number | null = null;
  cell.descendants((node, pos) => {
    if (textPos === null && node.isTextblock) {
      textPos = cellPos + 1 + pos + 1;
      return false;
    }
    return true;
  });

  if (textPos !== null) {
    tr.setSelection(TextSelection.create(tr.doc, textPos));
  }
}
