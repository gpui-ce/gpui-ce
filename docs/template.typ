#import "sidebar.typ": sidebar

#let template(current: str, doc) = {
  // Typst doesn't have file globbing so we have to add everything here and also in the rheo file oh well
  let site-nav = (
    (title: "Getting Started", items: (
      (id: "index", title: "Home", url: "./index.html"),
    )),
  )


  show: sidebar.with(
    nav: site-nav,
    current: current,
    title: "GPUI-CE",
    home-url: "./index.html",
  )

  doc
}
