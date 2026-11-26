p.deleted_at IS NULL AND (p.visibility IN ('local','public') OR p.user_id=$1 OR p.reply_user_id=$1 OR (p.id=$6 AND p.apub AND p.visibility='unlisted'))
        AND (p.channel_id IS NULL OR NOT c.is_private AND NOT c.is_dm AND c.deleted_at IS NULL)
        AND (NOT $4::boolean OR p.user_id=$1 OR p.reply_user_id=$1 OR EXISTS(SELECT 1 FROM channel_user WHERE channel_id=p.channel_id AND user_id=$1))
