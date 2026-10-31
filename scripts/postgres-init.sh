#!/bin/sh
set -eu
psql --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" --set=ON_ERROR_STOP=1 --set=app_password="$DATABASE_PASSWORD" <<'SQL'
CREATE ROLE ferry LOGIN PASSWORD :'app_password';
ALTER DATABASE ferry OWNER TO ferry;
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
GRANT ALL ON SCHEMA public TO ferry;
SQL
