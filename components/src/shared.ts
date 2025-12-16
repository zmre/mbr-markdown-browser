export const siteNav = fetch("/.mbr/site.json")
  .then((resp) => {
    return resp.json();
  })
