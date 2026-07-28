// @stencil-hq/wslab canonical scene-key composition. Generated modules
// re-export `itemKey` next to their `<Class>ItemKeys` constants so hosts
// never hand-assemble `each~item/relative` paths.

/** Join one `each` item into a full canonical scene key.
 *
 * `itemKey(each, item)` addresses the item root; `itemKey(each, item, rel)`
 * appends a template-relative key (both come from the generated
 * `<Class>ItemKeys` constants). `item` is the raw innermost item key exactly
 * as carried by signal `detail.item`; it is escaped here per the canonical
 * grammar (`%` → `%25`, `/` → `%2F`, `~` → `%7E`, `%` first). */
export function itemKey(each: string, item: string | number, rel = ''): string {
   const escaped = String(item).replace(/%/g, '%25').replace(/\//g, '%2F').replace(/~/g, '%7E');
   return rel === '' ? `${each}~${escaped}` : `${each}~${escaped}/${rel}`;
}
