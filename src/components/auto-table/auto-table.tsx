/** Port of 7link-mall-web AutoTable. Uses Ferry primitives and Chinese labels; no commerce/i18n runtime. */
import {
  Fragment,
  forwardRef,
  useImperativeHandle,
  useState,
  type ReactNode,
} from "react";
import { ChevronLeft, ChevronRight, FolderTree } from "lucide-react";
import { cn } from "cn";
import { Button } from "../ui/button";
import { Checkbox } from "../ui/checkbox";
import { evaluate } from "../auto-form";
import {
  autoTableViewportStyle,
  buildTableCsv,
  getActionColumnWidth,
  getPageSizeOptions,
  getSequenceNumber,
  getVisibleColumns,
} from "./auto-table-state";
import {
  isStickyColumn,
  stickyCellStyle,
  stickyColWidth,
} from "./table-sticky";
import {
  formatCellValue,
  getRowKey,
  type AutoTableHandle,
  type ColumnConfig,
  type TableOptions,
} from "./types";
import { useAutoTable } from "./use-auto-table";

const interactive =
  "a,button,input,select,textarea,[role=button],[role=checkbox],[role=combobox],[contenteditable=true]";
function AutoTableInner<T>(
  options: TableOptions<T>,
  ref: React.ForwardedRef<AutoTableHandle>,
) {
  const local = useAutoTable(options);
  const state = options.state;
  const rows = state?.rows ?? local.rows;
  const query = state?.query ?? local.query;
  const total = state?.total ?? local.total;
  const loading = state?.loading ?? options.loading ?? local.loading;
  const error = state?.error ?? local.error?.message;
  const selected =
    state?.selected ??
    new Set(local.selected.map((row) => getRowKey(row, options.rowKey)));
  const columns = getVisibleColumns(options.columns, options.hiddenFields);
  const pagination = options.pagination !== false;
  const setPage = options.onPageChange ?? local.setPage;
  const setPageSize = options.onPageSizeChange ?? local.setPageSize;
  const setSort = options.onSortChange ?? local.setSort;
  const retry = options.onRetry ?? local.refresh;
  const [collapsed, setCollapsed] = useState(new Set<string>());
  const hasActions = !!options.actionsContent || !!options.actions?.length;
  const sequenceOffset = options.showSequence ? 1 : 0;
  const offset = sequenceOffset + (options.selectable ? 1 : 0);
  const count = offset + columns.length + (hasActions ? 1 : 0);
  const actionWidth =
    options.actionWidth ?? getActionColumnWidth(options.actions?.length ?? 0);
  const widths = [
    ...(options.showSequence ? [56] : []),
    ...(options.selectable ? [40] : []),
    ...columns.map((column) => stickyColWidth(column.width)),
    ...(hasActions ? [actionWidth] : []),
  ];
  const style = (index: number, header = false) => ({
    ...(index < offset ||
    index >= offset + columns.length ||
    columns[index - offset]?.width
      ? { width: widths[index], minWidth: widths[index] }
      : {}),
    ...stickyCellStyle(index, count, widths, options.sticky, header),
  });
  const cellClass = (index: number, header = false) =>
    cn(
      "border-b border-border px-3 py-2.5 text-left",
      header
        ? "bg-muted text-xs font-medium text-muted-foreground"
        : "group-hover:bg-accent group-data-[selected=true]:bg-accent",
      !header && isStickyColumn(index, count, options.sticky) && "bg-card",
    );
  const allSelected =
    rows.length > 0 &&
    rows.every((row) => selected.has(getRowKey(row, options.rowKey)));
  const pages = Math.max(1, Math.ceil(total / query.pageSize));
  const clickable = (row: T) =>
    !!options.onRowClick && options.rowClickable?.(row) !== false;

  useImperativeHandle(
    ref,
    () => ({
      refresh: retry,
      exportCsv: (filename = "export.csv") => {
        const url = URL.createObjectURL(
          new Blob([buildTableCsv(columns, rows)], {
            type: "text/csv;charset=utf-8",
          }),
        );
        const anchor = document.createElement("a");
        anchor.href = url;
        anchor.download = filename;
        anchor.click();
        URL.revokeObjectURL(url);
      },
    }),
    [retry, columns, rows],
  );

  return (
    <div
      className={cn(
        "flex min-h-0 min-w-0 flex-col gap-3",
        options.fill && "flex-1",
        options.className,
      )}
      style={autoTableViewportStyle(options.fill, options.sticky?.maxHeight)}
      data-auto-table-root
    >
      {options.title && (
        <div className="flex items-center justify-between">{options.title}</div>
      )}
      <div
        ref={options.scrollRef}
        className="min-h-0 min-w-0 flex-1 overflow-auto rounded-lg border border-border bg-card"
        aria-busy={loading}
      >
        <table
          className={cn(
            "w-full border-separate border-spacing-0 text-sm",
            options.layout === "auto" ? "min-w-max table-auto" : "table-fixed",
          )}
          style={{
            minWidth:
              options.minWidth ?? widths.reduce((sum, width) => sum + width, 0),
          }}
        >
          <colgroup>
            {options.showSequence && <col style={{ width: 56 }} />}
            {options.selectable && <col style={{ width: 40 }} />}
            {columns.map((column) => (
              <col
                key={String(column.field)}
                style={column.width ? { width: column.width } : undefined}
              />
            ))}
            {hasActions && <col style={{ width: actionWidth }} />}
          </colgroup>
          <thead>
            <tr>
              {options.showSequence && (
                <th className={cellClass(0, true)} style={style(0, true)}>
                  序号
                </th>
              )}
              {options.selectable && (
                <th
                  className={cellClass(sequenceOffset, true)}
                  style={style(sequenceOffset, true)}
                >
                  <Checkbox
                    aria-label="全选"
                    checked={
                      allSelected
                        ? true
                        : rows.some((row) =>
                              selected.has(getRowKey(row, options.rowKey)),
                            )
                          ? "indeterminate"
                          : false
                    }
                    onCheckedChange={
                      options.onSelectAll ?? (() => local.selectAll(rows))
                    }
                  />
                </th>
              )}
              {columns.map((column, index) => (
                <th
                  key={String(column.field)}
                  className={cn(
                    cellClass(offset + index, true),
                    column.align === "right" && "text-right",
                    column.align === "center" && "text-center",
                  )}
                  style={style(offset + index, true)}
                  aria-sort={
                    query.sort?.field === column.field
                      ? query.sort.direction === "asc"
                        ? "ascending"
                        : "descending"
                      : undefined
                  }
                >
                  {column.sortable ? (
                    <button
                      type="button"
                      className="inline-flex items-center gap-1 rounded-sm text-left transition-colors hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
                      onClick={() => setSort(String(column.field))}
                    >
                      {evaluate(column.name, {})}
                      {query.sort?.field === column.field &&
                        (query.sort.direction === "asc" ? " ↑" : " ↓")}
                    </button>
                  ) : (
                    evaluate(column.name, {})
                  )}
                </th>
              ))}
              {hasActions && (
                <th
                  className={cn(cellClass(count - 1, true), "text-right")}
                  style={style(count - 1, true)}
                >
                  操作
                </th>
              )}
            </tr>
          </thead>
          <tbody>
            {loading && !rows.length
              ? Array.from({ length: 5 }, (_, row) => (
                  <tr key={row}>
                    {Array.from({ length: count }, (_, index) => (
                      <td key={index} className="border-b p-3">
                        <div className="h-4 animate-pulse rounded bg-muted" />
                      </td>
                    ))}
                  </tr>
                ))
              : rows.map((row, index) => {
                  const key = getRowKey(row, options.rowKey);
                  const group = options.groupBy?.(row);
                  const startsGroup =
                    group !== undefined &&
                    (!index || options.groupBy?.(rows[index - 1]!) !== group);
                  return (
                    <Fragment key={key}>
                      {startsGroup && (
                        <tr>
                          <td
                            colSpan={count}
                            className="border-b bg-muted/50 p-2"
                          >
                            <button
                              type="button"
                              className="flex w-full items-center gap-2 rounded px-1 py-1 text-left font-medium transition-colors hover:bg-accent focus-visible:outline-2 focus-visible:outline-ring"
                              aria-expanded={!collapsed.has(group)}
                              onClick={() =>
                                setCollapsed((current) => {
                                  const next = new Set(current);
                                  if (next.has(group)) next.delete(group);
                                  else next.add(group);
                                  return next;
                                })
                              }
                            >
                              <ChevronRight
                                className={cn(
                                  "size-4 transition-transform",
                                  !collapsed.has(group) && "rotate-90",
                                )}
                              />
                              <FolderTree className="size-4" />
                              {group}
                              <span className="text-xs font-normal text-muted-foreground">
                                {
                                  rows.filter(
                                    (item) => options.groupBy?.(item) === group,
                                  ).length
                                }{" "}
                                项 · 当前页
                              </span>
                            </button>
                          </td>
                        </tr>
                      )}
                      <tr
                        hidden={group !== undefined && collapsed.has(group)}
                        data-selected={selected.has(key)}
                        className={cn(
                          "group transition-colors data-[selected=true]:bg-accent",
                          clickable(row) &&
                            "cursor-pointer focus-visible:outline-2 focus-visible:outline-ring",
                          options.rowClassName?.(row),
                        )}
                        tabIndex={clickable(row) ? 0 : undefined}
                        onClick={(event) => {
                          if (
                            clickable(row) &&
                            !(event.target as Element).closest(interactive)
                          )
                            options.onRowClick?.(row);
                        }}
                        onKeyDown={(event) => {
                          if (
                            clickable(row) &&
                            event.target === event.currentTarget &&
                            ["Enter", " "].includes(event.key)
                          ) {
                            event.preventDefault();
                            options.onRowClick?.(row);
                          }
                        }}
                      >
                        {options.showSequence && (
                          <td
                            className={cn(
                              cellClass(0),
                              "tabular-nums text-muted-foreground",
                            )}
                            style={style(0)}
                          >
                            {getSequenceNumber(
                              options.pagination === "next" ? 1 : query.page,
                              query.pageSize,
                              index,
                            )}
                          </td>
                        )}
                        {options.selectable && (
                          <td
                            className={cellClass(sequenceOffset)}
                            style={style(sequenceOffset)}
                            onClick={(event) => event.stopPropagation()}
                          >
                            <Checkbox
                              aria-label="选择此行"
                              checked={selected.has(key)}
                              onCheckedChange={() =>
                                options.onToggleSelect
                                  ? options.onToggleSelect(row)
                                  : local.toggleSelect(row)
                              }
                            />
                          </td>
                        )}
                        {columns.map((column, ci) => (
                          <td
                            key={String(column.field)}
                            className={cn(
                              cellClass(offset + ci),
                              "break-words",
                              column.align === "right" && "text-right",
                              column.align === "center" && "text-center",
                            )}
                            style={style(offset + ci)}
                            onClick={
                              column.onclick
                                ? (event) => {
                                    event.stopPropagation();
                                    column.onclick?.(row, event);
                                  }
                                : undefined
                            }
                          >
                            <CellContent row={row} column={column} />
                          </td>
                        ))}
                        {hasActions && (
                          <td
                            className={cn(
                              cellClass(count - 1),
                              "text-right whitespace-nowrap",
                            )}
                            style={style(count - 1)}
                            onClick={(event) => event.stopPropagation()}
                          >
                            {options.actionsContent ? (
                              options.actionsContent(row)
                            ) : (
                              <div className="flex items-center justify-end gap-1">
                                {options.actions
                                  ?.filter(
                                    (action) => action.show?.(row) !== false,
                                  )
                                  .map((action, index) => {
                                    const name = evaluate(action.name, {});
                                    const Icon = action.icon;
                                    return (
                                      <Button
                                        type="button"
                                        key={index}
                                        variant="ghost"
                                        size="icon"
                                        className="size-8"
                                        title={
                                          typeof name === "string"
                                            ? name
                                            : undefined
                                        }
                                        aria-label={
                                          typeof name === "string"
                                            ? name
                                            : undefined
                                        }
                                        disabled={action.disabled?.(row)}
                                        style={{ color: action.color }}
                                        onClick={() => action.onclick?.(row)}
                                      >
                                        {Icon ? (
                                          <Icon className="size-4" />
                                        ) : (
                                          name
                                        )}
                                      </Button>
                                    );
                                  })}
                              </div>
                            )}
                          </td>
                        )}
                      </tr>
                    </Fragment>
                  );
                })}
            {!loading && !rows.length && (
              <tr>
                <td
                  colSpan={count}
                  className="px-4 py-16 text-center text-muted-foreground"
                >
                  {error ? (
                    <div role="alert" className="space-y-3 text-destructive">
                      <p>{error}</p>
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        onClick={retry}
                      >
                        重试
                      </Button>
                    </div>
                  ) : (
                    (options.empty ?? "没有符合条件的记录。")
                  )}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      {pagination && (
        <div
          className="flex shrink-0 flex-wrap items-center gap-3 pb-2 text-sm"
          data-auto-table-footer
        >
          {options.footerContent}
          <span className="flex-1" />
          <span className="text-muted-foreground">
            {options.pagination === "next"
              ? `第 ${query.page} 页 · 本页 ${rows.length} 条`
              : `共 ${total} 条 · ${total ? (query.page - 1) * query.pageSize + 1 : 0}–${Math.min(query.page * query.pageSize, total)}`}
          </span>
          <nav className="flex items-center gap-2" aria-label="分页">
            <select
              aria-label="每页条数"
              className="h-8 rounded-md border border-input bg-background px-2 text-sm"
              value={query.pageSize}
              onChange={(event) => setPageSize(Number(event.target.value))}
            >
              {getPageSizeOptions(
                query.pageSize,
                options.pageSizeOptions ?? [10, 20, 50, 100, 200],
              ).map((size) => (
                <option key={size} value={size}>
                  {size} 条/页
                </option>
              ))}
            </select>
            <Button
              type="button"
              variant="outline"
              size="icon"
              className="size-8"
              aria-label="上一页"
              disabled={query.page <= 1 || loading}
              onClick={() => setPage(query.page - 1)}
            >
              <ChevronLeft />
            </Button>
            <span className="tabular-nums">
              {options.pagination === "next"
                ? query.page
                : `${query.page} / ${pages}`}
            </span>
            <Button
              type="button"
              variant="outline"
              size="icon"
              className="size-8"
              aria-label="下一页"
              disabled={
                loading ||
                !!error ||
                (options.pagination === "next"
                  ? !options.hasMore
                  : query.page >= pages)
              }
              onClick={() => setPage(query.page + 1)}
            >
              <ChevronRight />
            </Button>
          </nav>
        </div>
      )}
    </div>
  );
}
function CellContent<T>({
  row,
  column,
}: {
  row: T;
  column: ColumnConfig<T>;
}): ReactNode {
  const value = (row as Record<string, unknown>)[String(column.field)];
  if (column.render) return column.render(value, row, { column });
  return (
    <span
      style={{
        color:
          typeof column.color === "function"
            ? column.color(value, row)
            : column.color,
      }}
    >
      {formatCellValue(row, column) ?? "—"}
    </span>
  );
}
export const AutoTable = forwardRef(AutoTableInner) as <T>(
  props: TableOptions<T> & React.RefAttributes<AutoTableHandle>,
) => React.ReactElement;
