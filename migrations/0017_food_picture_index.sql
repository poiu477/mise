-- Photos follow their version's visibility: /media serves a photo to a household that can see a version using it,
-- and a food's picture to everyone. This finds whether a photo is some food's picture without reading every food.
CREATE INDEX foods_image_media ON foods ((props->>'image_media_id'));
