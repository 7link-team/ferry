import { useCallback, useEffect, useRef, useState } from "react";

import {
  getRowKey,
  type TableOptions,
  type TableQuery,
  type UseAutoTableReturn,
} from "./types";
import {
  clampPageToTotal,
  getPageSizeOptions,
  syncExternalFilter,
} from "./auto-table-state";

const DEFAULT_PAGE_SIZE = 20;
const DEFAULT_PAGE_SIZE_OPTIONS = [10, 20, 50, 100];

function sortLocalData<T>(rows: T[], sort: TableQuery["sort"]): T[] {
  if (!sort) return rows;
  const { field, direction } = sort;
  const multiplier = direction === "asc" ? 1 : -1;
  return [...rows].sort((a, b) => {
    const av = (a as Record<string, unknown>)[field];
    const bv = (b as Record<string, unknown>)[field];
    if (av == null && bv == null) return 0;
    if (av == null) return 1 * multiplier;
    if (bv == null) return -1 * multiplier;
    if (typeof av === "number" && typeof bv === "number") {
      return (av - bv) * multiplier;
    }
    return String(av).localeCompare(String(bv)) * multiplier;
  });
}

export function useAutoTable<T>(
  options: TableOptions<T>,
): UseAutoTableReturn<T> {
  const {
    data,
    fetch,
    selection = "multiple",
    pageSize = DEFAULT_PAGE_SIZE,
    pageSizeOptions: configuredPageSizeOptions = DEFAULT_PAGE_SIZE_OPTIONS,
    defaultSort,
    initialQuery,
    filter,
    onSelectionChange,
    onQueryChange,
  } = options;
  const initialPageSize = initialQuery?.pageSize ?? pageSize;
  const pageSizeOptions = getPageSizeOptions(
    initialPageSize,
    configuredPageSizeOptions,
  );

  const [query, setQuery] = useState<TableQuery>({
    page: initialQuery?.page ?? 1,
    pageSize: pageSizeOptions.includes(initialPageSize)
      ? initialPageSize
      : (pageSizeOptions[0] ?? DEFAULT_PAGE_SIZE),
    sort:
      initialQuery?.sort ??
      (defaultSort
        ? { field: defaultSort.field, direction: defaultSort.direction }
        : undefined),
    filter: initialQuery?.filter ?? filter ?? {},
  });

  const [rows, setRows] = useState<T[]>(data ?? []);
  const [total, setTotal] = useState(data?.length ?? 0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<Error | null>(null);
  const [selected, setSelected] = useState<T[]>([]);
  const fetchSeq = useRef(0);
  const queryRef = useRef(query);
  queryRef.current = query;

  const isLocal = data !== undefined;

  const fetchData = useCallback(async () => {
    if (options.state) return;
    if (isLocal) {
      // Local mode: sort then slice to the current page; total is the full length (only pagination is real)
      const sorted = sortLocalData(data, query.sort);
      const page = clampPageToTotal(query.page, query.pageSize, sorted.length);
      const start = (page - 1) * query.pageSize;
      setRows(sorted.slice(start, start + query.pageSize));
      setTotal(sorted.length);
      if (page !== query.page) {
        const next = { ...queryRef.current, page };
        onQueryChange?.(next);
        queryRef.current = next;
        setQuery(next);
      }
      return;
    }
    if (!fetch) return;
    const seq = fetchSeq.current + 1;
    fetchSeq.current = seq;
    setLoading(true);
    setError(null);
    try {
      const res = await fetch(query);
      if (fetchSeq.current !== seq) return;
      setRows(res.rows);
      setTotal(res.total);
    } catch (err) {
      if (fetchSeq.current !== seq) return;
      setError(err instanceof Error ? err : new Error(String(err)));
    } finally {
      if (fetchSeq.current === seq) {
        setLoading(false);
      }
    }
  }, [data, fetch, isLocal, query, options.state]);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void fetchData();
  }, [fetchData]);

  const commitQuery = useCallback(
    (update: (previous: TableQuery) => TableQuery) => {
      const next = update(queryRef.current);
      onQueryChange?.(next);
      queryRef.current = next;
      setQuery(next);
    },
    [onQueryChange],
  );

  const filterKey = JSON.stringify(filter ?? {});
  useEffect(() => {
    if (!options.state)
      commitQuery((previous) => syncExternalFilter(previous, filter ?? {}));
    // filterKey tracks the external value rather than the caller's object identity.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [filterKey, commitQuery, filter]);

  const setPage = useCallback(
    (page: number) => {
      commitQuery((prev) => ({ ...prev, page: Math.max(1, page) }));
    },
    [commitQuery],
  );

  const setPageSize = useCallback(
    (size: number) => {
      commitQuery((prev) => ({ ...prev, pageSize: size, page: 1 }));
    },
    [commitQuery],
  );

  const setSort = useCallback(
    (field: string) => {
      commitQuery((prev) => {
        const current = prev.sort;
        let nextSort: TableQuery["sort"];
        if (current?.field === field) {
          if (current.direction === "asc") {
            nextSort = { field, direction: "desc" };
          } else {
            nextSort = undefined;
          }
        } else {
          nextSort = { field, direction: "asc" };
        }
        return { ...prev, sort: nextSort, page: 1 };
      });
    },
    [commitQuery],
  );

  const setFilter = useCallback(
    (nextFilter: Record<string, unknown>) => {
      commitQuery((prev) => ({ ...prev, filter: nextFilter, page: 1 }));
    },
    [commitQuery],
  );

  const refresh = useCallback(() => {
    void fetchData();
  }, [fetchData]);

  const toggleSelect = useCallback(
    (row: T) => {
      setSelected((prev) => {
        const key = getRowKey(row, options.rowKey);
        const exists = prev.some((r) => getRowKey(r, options.rowKey) === key);
        const next =
          selection === "single"
            ? exists
              ? []
              : [row]
            : exists
              ? prev.filter((r) => getRowKey(r, options.rowKey) !== key)
              : [...prev, row];
        onSelectionChange?.(next);
        return next;
      });
    },
    [onSelectionChange, options.rowKey, selection],
  );

  const selectAll = useCallback(
    (pageRows: T[]) => {
      setSelected((prev) => {
        const keys = new Set(prev.map((r) => getRowKey(r, options.rowKey)));
        const allSelected = pageRows.every((r) =>
          keys.has(getRowKey(r, options.rowKey)),
        );
        const next = allSelected
          ? prev.filter(
              (r) =>
                !pageRows.some(
                  (pr) =>
                    getRowKey(pr, options.rowKey) ===
                    getRowKey(r, options.rowKey),
                ),
            )
          : [
              ...prev,
              ...pageRows.filter(
                (r) => !keys.has(getRowKey(r, options.rowKey)),
              ),
            ];
        onSelectionChange?.(next);
        return next;
      });
    },
    [onSelectionChange, options.rowKey],
  );

  return {
    rows,
    total,
    loading,
    error,
    query,
    selected,
    setPage,
    setPageSize,
    setSort,
    setFilter,
    toggleSelect,
    selectAll,
    refresh,
  };
}
