//! The one implementation of "list a resource": filtering, search, sorting,
//! pagination and the response envelope.
//!
//! Every resource declares a `Spec` and calls `list`. Nothing builds its own
//! `WHERE` clause, because this is the SQL-injection boundary: column names
//! are only ever taken from the `Spec`'s whitelists, never from the request,
//! and every value is a bound parameter.

use crate::snowflake::Id;

use crate::error::{ApiError, Result};
use sea_orm::sea_query::{Expr, ExprTrait, Func};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, EntityTrait, FromQueryResult, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, Select, Value,
};
use std::collections::HashMap;
use std::str::FromStr;

pub const DEFAULT_PAGE_SIZE: u64 = 20;
pub const MAX_PAGE_SIZE: u64 = 200;

/// What a resource allows to be searched, filtered and sorted by.
pub struct Spec {
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
        let page_size = std::cmp::min(number("page_size", DEFAULT_PAGE_SIZE)?, MAX_PAGE_SIZE);

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

/// Resolve only validated Spec names to the entity's typed column enum.
fn column<E: EntityTrait>(name: &str) -> E::Column {
    E::Column::from_str(name).unwrap_or_else(|_| panic!("invalid resource column: {name}"))
}

fn conditions<E: EntityTrait>(
    spec: &Spec,
    workspace_id: &Id,
    query: &ListQuery,
) -> Result<Condition> {
    Ok(Condition::all()
        .add(column::<E>("workspace_id").eq(*workspace_id))
        .add(filters::<E>(spec, query)?))
}

fn filters<E: EntityTrait>(spec: &Spec, query: &ListQuery) -> Result<Condition> {
    let mut condition = Condition::all();
    if let Some(text) = &query.search {
        let mut search = Condition::any();
        for name in spec.search {
            search = search.add(
                Expr::expr(Func::lower(Expr::col((E::default(), column::<E>(name)))))
                    .like(format!("%{}%", text.to_lowercase())),
            );
        }
        condition = condition.add(search);
    }
    for (name, options) in &query.filters {
        let values: Vec<Value> = options
            .iter()
            .map(|option| {
                if name.ends_with("_id") {
                    Ok(Id::reference(option)?.into())
                } else {
                    Ok(option.clone().into())
                }
            })
            .collect::<Result<_>>()?;
        condition = condition.add(column::<E>(name).is_in(values));
    }
    Ok(condition)
}

/// Reads one page plus its total, using the same validated conditions.
pub async fn list<E: EntityTrait, M: FromQueryResult + Send + Sync>(
    conn: &impl ConnectionTrait,
    spec: &Spec,
    select: Select<E>,
    workspace_id: &Id,
    query: &ListQuery,
) -> Result<(Vec<M>, u64)> {
    let select = select.filter(conditions::<E>(spec, workspace_id, query)?);
    page(conn, select, query).await
}

/// Deliberately global: caller must have obtained a platform Admin extractor.
pub async fn global_list<E: EntityTrait, M: FromQueryResult + Send + Sync>(
    conn: &impl ConnectionTrait,
    _admin: &crate::platform::Admin,
    spec: &Spec,
    select: Select<E>,
    query: &ListQuery,
) -> Result<(Vec<M>, u64)> {
    page(conn, select.filter(filters::<E>(spec, query)?), query).await
}

async fn page<E: EntityTrait, M: FromQueryResult + Send + Sync>(
    conn: &impl ConnectionTrait,
    select: Select<E>,
    query: &ListQuery,
) -> Result<(Vec<M>, u64)> {
    let total = select.clone().into_model::<M>().count(conn).await?;
    let direction = if query.descending {
        sea_orm::Order::Desc
    } else {
        sea_orm::Order::Asc
    };
    let rows = select
        .order_by(column::<E>(&query.sort), direction)
        .order_by_asc(column::<E>("id"))
        .limit(query.page_size)
        .offset((query.page - 1) * query.page_size)
        .into_model::<M>()
        .all(conn)
        .await?;
    Ok((rows, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: Spec = Spec {
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
        use crate::entities::hosts;
        use sea_orm::QueryTrait;
        let statement = hosts::Entity::find()
            .filter(conditions::<hosts::Entity>(&SPEC, &Id::from(1), &q).unwrap())
            .build(sea_orm::DbBackend::Postgres);
        let sql = statement.sql;
        let values = statement.values.unwrap().0;
        assert!(sql.contains("\"workspace_id\" = $1"));
        assert!(
            sql.contains(
                "LOWER(\"hosts\".\"name\") LIKE $2 OR LOWER(\"hosts\".\"address\") LIKE $3"
            )
        );
        assert!(sql.contains("\"state\" IN ($4, $5)"));
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
        use crate::entities::hosts;
        use sea_orm::QueryTrait;
        let statement = hosts::Entity::find()
            .filter(conditions::<hosts::Entity>(&SPEC, &Id::from(1), &q).unwrap())
            .build(sea_orm::DbBackend::Postgres);
        assert!(
            statement
                .sql
                .contains("WHERE \"hosts\".\"workspace_id\" = $1")
        );
        assert_eq!(statement.values.unwrap().0, vec![1_i64.into()]);
    }
}
