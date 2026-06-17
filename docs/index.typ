#import "sidebar.typ": sidebar

// Typst doesn't have file globbing so we have to add everything here and also in the rheo file oh well
#let site-nav = (
  (title: "Getting Started", items: (
    (id: "index", title: "Home", url: "./index.html"),
  )),
)

#show: sidebar.with(
  nav: site-nav,
  current: "index",
  title: "GPUI-CE",
  home-url: "./index.html",
)

= Hello World!

Welcome to the GPUI-CE documentation.
