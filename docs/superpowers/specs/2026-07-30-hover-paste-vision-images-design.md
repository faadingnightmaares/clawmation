# Hover-to-Paste Vision Images

## Goal

Let users paste an image with Ctrl+V while the pointer is over a vision image
gallery. The pasted image becomes the next primary or alternative state through
the same storage and validation path already used by drag and drop.

## Scope

- Apply the behavior to the shared image gallery used by Loops, Watch, and the
  trigger editor.
- Activate paste only while the pointer is inside that gallery.
- Accept the first clipboard item whose MIME type starts with `image/`.
- Preserve the existing 20 MB limit and eight-image maximum.
- Ignore clipboard content that contains no image.
- Preserve file picker, drag and drop, magic selection, removal, thumbnails,
  ordering, and saved Loop/Watch formats.

## Design

`ImageCandidateGallery` will listen for the window `paste` event while mounted,
but will forward an image only when its own hover flag is active and the gallery
is neither busy nor full. It will prevent the browser's default paste behavior
only after finding an acceptable image.

The gallery will expose the pasted `File` through the same callback used for a
dropped file. Each editor will therefore keep using its existing MIME check,
size check, base64 conversion, `save_template_upload` command, candidate
deduplication, thumbnail update, and notification behavior. No Rust or file
format change is required.

## Error Handling

- Non-image clipboard data is ignored without a notification.
- Unsupported or oversized image files use the existing import errors.
- A busy or full gallery ignores paste, matching its disabled add controls.
- Backend save failures use the editor's existing import-failure notification.

## Verification

- Paste over a gallery imports one image and persists its thumbnail.
- Paste outside the gallery does nothing.
- Text-only clipboard content does nothing.
- A gallery containing eight images does not import a ninth.
- Existing drag/drop and gallery tests continue to pass.
