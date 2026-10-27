-- Folded search text per pull request (title, description, repo, number,
-- author, branch). NULL rows are filled in by the store after migrating.
ALTER TABLE pull_requests ADD COLUMN search_text TEXT;
-- Project sections and project-name search look up links by project.
CREATE INDEX pr_projects_by_project ON pr_projects(project_id);
