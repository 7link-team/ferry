//! The one implementation of "list a resource": filtering, search, sorting,
//! pagination and the response envelope.
//!
//! Every resource declares a `Spec` and calls `list`. Nothing builds its own
//! `WHERE` clause, because this is the SQL-injection boundary: column names
//! are only ever taken from the `Spec`'s whitelists, never from the request,
//! and every value is a bound parameter.

use crate::{
    db,
    error::{ApiError, Result},
};
use sea_orm::{ConnectionTrait, QueryResult, Value};
use std::collections::HashMap;

pub const DEFAULT_PAGE_SIZE: u64 = 20;
pub const MAX_PAGE_SIZE: u64 = 200;

/// What a resource allows to be searched, filtered and sorted by.
pub struct Spec {
    pub table: &'static str,
    /// Columns that `?q=` matches with a case-insensitive prefix/contains scan.
    pub search: &'static [&'static str],
    /// Columns usable as `?column=value`. A comma in the value means "any of".
    pub filters: &'static [&'static str],
    pub sortable: &'static [&'static str],
    /// Applied when the request has no `?sort=`.
    pub default_sort: &'static str,
}

/// The query string of a list request, already validated against a `Spec`.
pub struct ListQuery {
    pub page: u64,
    pub page_size: u64,
    pub sort: String,
    pub descending: bool,
    pub search: Option<String>,
    pub filters: Vec<(String, Vec<String>)>,
}

impl ListQuery {
    /// Rejects unknown columns rather than ignoring them: a filter the server
    /// silently drops shows the operator a complete list of the wrong rows.
    pub fn parse(spec: &Spec, raw: &HashMap<String, String>) -> Result<Self> {
        let number = |key: &str, fallback: u64| -> Result<u64> {
            match raw.get(key) {
                None => Ok(fallback),
                Some(text) => text
                    .parse::<u64>()
                    .ok()
                    .filter(|v| *v > 0)
                    .ok_or_else(|| ApiError::bad(format!("{key} 必须是正整数。"))),
            }
        };
        let page = number("page", 1)?;
        let page_size = number("page_size", DEFAULT_PAGE_SIZE)?.min(MAX_PAGE_SIZE);

        let requested = raw
            .get("sort")
            .map(String::as_str)
            .unwrap_or(spec.default_sort);
        let descending = requested.starts_with('-');
        let sort = requested.trim_start_matches('-').to_owned();
        if !spec.sortable.contains(&sort.as_str()) {
            return Err(ApiError::bad(format!("不支持按 {sort} 排序。")));
        }

        let search = raw
            .get("q")
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty() && !spec.search.is_empty());

        let mut filters = Vec::new();
        for (key, value) in raw {
            if matches!(
                key.as_str(),
                "page" | "page_size" | "sort" | "q" | "at" | "format"
            ) {
                continue;
            }
            if !spec.filters.contains(&key.as_str()) {
                return Err(ApiError::bad(format!("不支持按 {key} 过滤。")));
            }
            let values: Vec<String> = value
                .split(',')
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
                .collect();
            if !values.is_empty() {
                filters.push((key.clone(), values));
            }
        }
        Ok(Self {
            page,
            page_size,
            sort,
            descending,
            search,
            filters,
        })
    }
}

/// `WHERE` fragment plus its bound values, shared by the count and page reads.
fn conditions(spec: &Spec, workspace_id: &str, query: &ListQuery) -> (String, Vec<Value>) {
    let mut sql = String::from(" WHERE workspace_id = ?");
    let mut values: Vec<Value> = vec![workspace_id.into()];
    if let Some(text) = &query.search {
        let clauses: Vec<String> = spec
            .search
            .iter()
            .map(|column| format!("LOWER({column}) LIKE ?"))
            .collect();
        sql.push_str(&format!(" AND ({})", clauses.join(" OR ")));
        let pattern = format!("%{}%", text.to_lowercase());
        for _ in spec.search {
            values.push(pattern.clone().into());
        }
    }
    for (column, options) in &query.filters {
        let slots = vec!["?"; options.len()].join(",");
        sql.push_str(&format!(" AND {column} IN ({slots})"));
        for option in options {
            values.push(option.clone().into());
        }
    }
    (sql, values)
}

/// Reads one page plus the total. `columns` is a literal select list from the
/// resource module, never anything the client sent.
pub async fn list(
    conn: &impl ConnectionTrait,
    spec: &Spec,
    columns: &str,
    workspace_id: &str,
    query: &ListQuery,
) -> Result<(Vec<QueryResult>, u64)> {
    let (where_sql, values) = conditions(spec, workspace_id, query);

    let total = db::one(
        conn,
        &format!("SELECT COUNT(*) AS total FROM {}{where_sql}", spec.table),
        values.clone(),
    )
    .await?
    .map(|row| db::int(&row, "total") as u64)
    .unwrap_or(0);

    // A page past the end returns an empty page rather than an error: a shared
    // link stays valid after rows are deleted.
    let direction = if query.descending { "DESC" } else { "ASC" };
    let mut paged = values;
    paged.push((query.page_size as i64).into());
    paged.push((((query.page - 1) * query.page_size) as i64).into());
    let rows = db::all(
        conn,
        &format!(
            "SELECT {columns} FROM {}{where_sql} ORDER BY {} {direction}, id ASC LIMIT ? OFFSET ?",
            spec.table, query.sort
        ),
        paged,
    )
    .await?;
    Ok((rows, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: Spec = Spec {
        table: "hosts",
        search: &["name", "address"],
        filters: &["state", "group_id"],
        sortable: &["name", "created_at"],
        default_sort: "-created_at",
    };

    fn raw(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn defaults_and_descending_sort() {
        let q = ListQuery::parse(&SPEC, &raw(&[])).unwrap();
        assert_eq!((q.page, q.page_size), (1, DEFAULT_PAGE_SIZE));
        assert_eq!(q.sort, "created_at");
        assert!(q.descending);
    }

    #[test]
    fn page_size_is_capped_and_unknown_columns_are_refused() {
        let q = ListQuery::parse(&SPEC, &raw(&[("page_size", "9999")])).unwrap();
        assert_eq!(q.page_size, MAX_PAGE_SIZE);
        assert!(ListQuery::parse(&SPEC, &raw(&[("sort", "password")])).is_err());
        assert!(ListQuery::parse(&SPEC, &raw(&[("evil", "1")])).is_err());
        for bad in ["0", "-1", "abc", ""] {
            assert!(
                ListQuery::parse(&SPEC, &raw(&[("page", bad)])).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn every_value_is_bound_and_columns_come_only_from_the_spec() {
        let q =
            ListQuery::parse(&SPEC, &raw(&[("q", "web"), ("state", "online,offline")])).unwrap();
        let (sql, values) = conditions(&SPEC, "ws-1", &q);
        assert_eq!(
            sql,
            " WHERE workspace_id = ? AND (LOWER(name) LIKE ? OR LOWER(address) LIKE ?) AND state IN (?,?)"
        );
        // workspace + two search patterns + two state options.
        assert_eq!(values.len(), 5);
        assert!(!sql.contains("web") && !sql.contains("online"));
    }

    #[test]
    fn filters_never_escape_the_workspace() {
        // A client-supplied workspace_id filter is not in the whitelist, so it
        // cannot widen the scope.
        assert!(ListQuery::parse(&SPEC, &raw(&[("workspace_id", "other")])).is_err());
        let q = ListQuery::parse(&SPEC, &raw(&[])).unwrap();
        assert!(
            conditions(&SPEC, "ws-1", &q)
                .0
                .starts_with(" WHERE workspace_id = ?")
        );
    }
}
