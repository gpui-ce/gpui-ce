#let sidebar(
  nav: (),
  current: none,
  title: "",
  home-url: "/",
  doc,
) = {
  context if target() == "html" {
    html.elem("input", attrs: ("type": "checkbox", "id": "t"), [])
    html.elem("label", attrs: ("for": "t"), [])
    html.elem("div", attrs: ("id": "l"), [
      #html.elem("nav", [
        #html.elem("header", [
          #html.elem("a", attrs: ("href": home-url), [
             #html.elem("h1", [#title])
          ])
        ])
        #html.elem("ul", [
          #for section in nav {
            html.elem("li", [
              #html.elem("span", [#section.title])
            ])
            for item in section.at("items", default: ()) {
              let attrs = if item.id == current {
                ("href": item.url, "aria-current": "page")
              } else {
                ("href": item.url,)
              }
              html.elem("li", [
                #html.elem("a", attrs: attrs, [#item.title])
              ])
            }
          }
        ])
      ])
      #html.elem("main", [#doc])
    ])
    html.elem("label", attrs: ("for": "t"), [])
  } else {
    doc
  }
}
