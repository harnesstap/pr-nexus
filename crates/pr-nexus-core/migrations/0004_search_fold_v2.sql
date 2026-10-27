-- Search folding now also maps dash variants to '-' and compatibility forms
-- (full-width letters, digits, '#') to their plain form. Clear the stored
-- text so the store rebuilds it with the new fold right after migrating.
UPDATE pull_requests SET search_text = NULL;
