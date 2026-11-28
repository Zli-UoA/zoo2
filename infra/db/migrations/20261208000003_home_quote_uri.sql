-- Store only the target accepted by the quote creation endpoint.
ALTER TABLE home_posts ADD COLUMN quote_uri text;
