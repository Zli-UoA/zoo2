-- Visibility and federation are independent choices in the Home composer.
-- Legacy 'local' stays valid for non-federated API clients.
ALTER TABLE home_posts DROP CONSTRAINT home_posts_audience;
ALTER TABLE home_posts ADD CONSTRAINT home_posts_audience
  CHECK (NOT apub OR visibility <> 'local');
