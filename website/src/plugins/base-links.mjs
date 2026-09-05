import { defineMdastPlugin } from 'satteri';

/**
 * Sätteri (Astro 7 Markdown processor) plugin.
 *
 * Rewrites root-relative links (`/api/rest/`) to include Astro's `base`
 * (`/Kizuna-Docs/api/rest/`) so content can be written without hard-coding the
 * deployment prefix. Absolute URLs, anchors, mailto: and links that already
 * carry the base are left untouched. Also handles `href`/`link` string
 * attributes on MDX components such as <LinkCard href="/api/rest/" />.
 */
export function baseLinks({ base }) {
  const prefix = (base ?? '/').replace(/\/$/, '');
  const rewrite = (url) => {
    if (!prefix || typeof url !== 'string') return url;
    if (!url.startsWith('/') || url.startsWith('//')) return url;
    if (url === prefix || url.startsWith(prefix + '/')) return url;
    return prefix + url;
  };
  const fixAttrs = (node, ctx) => {
    const attrs = node.attributes ?? [];
    let changed = false;
    const next = attrs.map((attr) => {
      if (attr.type === 'mdxJsxAttribute' && (attr.name === 'href' || attr.name === 'link') && typeof attr.value === 'string') {
        const v = rewrite(attr.value);
        if (v !== attr.value) {
          changed = true;
          return { ...attr, value: v };
        }
      }
      return attr;
    });
    // `attributes` isn't a settable property, so swap the whole node for a
    // shallow copy carrying the rewritten attribute list.
    if (changed) ctx.replaceNode(node, { ...node, attributes: next });
  };
  return defineMdastPlugin({
    name: 'kizuna-base-links',
    link(node, ctx) {
      const v = rewrite(node.url);
      if (v !== node.url) ctx.setProperty(node, 'url', v);
    },
    definition(node, ctx) {
      const v = rewrite(node.url);
      if (v !== node.url) ctx.setProperty(node, 'url', v);
    },
    mdxJsxFlowElement: fixAttrs,
    mdxJsxTextElement: fixAttrs,
  });
}
