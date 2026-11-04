-- psql -XAt -v authz_issuer=https://account.zli.works/api/auth -f scripts/export-custom-emojis.sql > export/rows.json
SELECT coalesce(jsonb_agg(jsonb_build_object(
  'name', e.name, 'category', e.category, 'aliases', e.aliases, 'url', e.url, 'created_by', e.created_by,
  'identities', coalesce((SELECT jsonb_agg(jsonb_build_object('iss',i.iss,'sub',i.sub))
    FROM user_identities i WHERE i.user_id=e.created_by AND i.iss=:'authz_issuer'), '[]'::jsonb)
) ORDER BY e.name), '[]'::jsonb) FROM workspace_emojis e;
