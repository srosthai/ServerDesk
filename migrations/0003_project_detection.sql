-- Laravel project detection stores only bounded, non-secret metadata.
-- Secrets from .env are never read into the database or logs.

ALTER TABLE projects ADD COLUMN laravel_version TEXT;
ALTER TABLE projects ADD COLUMN php_version TEXT;
ALTER TABLE projects ADD COLUMN queue_hint TEXT;
ALTER TABLE projects ADD COLUMN scheduler_hint TEXT;
