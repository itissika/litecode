# SQLPage

Turn SQL queries into a small local web app. The managed install downloads the
pinned SQLPage Windows build and creates a starter site inside the workspace.

- Pages are plain `.sql` files under `.litecode/apps/sqlpage/site`. `index.sql` is the homepage; add `reports.sql` and open `/reports.sql` to add a page.
- The site uses a SQLite database at `.litecode/apps/sqlpage/sqlpage.db`.
- The server listens on loopback only; LiteCode verifies that on start.
- Edit the SQL files while the app is running; development mode reloads them on each request.
- Uninstall removes the downloaded runtime but keeps the site and database.
