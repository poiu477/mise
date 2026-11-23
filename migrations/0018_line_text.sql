-- Recipes written in the app keep each ingredient line as it was typed ("2 cups flour, sifted"), beside the parts
-- read from it (the name in raw_name, the note, the amount in line_amounts), so the editor shows a line again
-- exactly as its writer left it. Null for lines that came as parts: a seed bundle's.
ALTER TABLE ingredient_lines ADD COLUMN text text;
