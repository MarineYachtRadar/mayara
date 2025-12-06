import van from "./van-1.5.2.debug.js";
import { fetchRadars, fetchInterfaces, isStandaloneMode, detectMode } from "./api.js";

const { a, tr, td, div, p, strong, details, summary } = van.tags;

// Network requirements for different radar brands
const NETWORK_REQUIREMENTS = {
  furuno: {
    ipRange: "172.31.x.x/16",
    description: "Furuno DRS radars require the host to have an IP address in the 172.31.x.x range. " +
      "Configure your network interface with an IP like 172.31.3.x/16 on the interface connected to the radar network.",
    example: "Example: ip addr add 172.31.3.100/16 dev eth1"
  },
  navico: {
    ipRange: "236.6.7.x (multicast)",
    description: "Navico (Simrad/Lowrance/B&G) radars use multicast. Ensure your network supports multicast routing.",
  },
  raymarine: {
    ipRange: "232.1.1.x (multicast)",
    description: "Raymarine radars use multicast. Ensure your network supports multicast routing.",
  },
  garmin: {
    ipRange: "239.254.2.x (multicast)",
    description: "Garmin xHD radars use multicast. Ensure your network supports multicast routing.",
  }
};

const RadarEntry = (id, name) =>
  tr(
    td(
      { class: "myr" },
      a({ href: "control.html?id=" + id }, name + " controller")
    ),
    td(
      { class: "myr" },
      a({ href: "viewer.html?id=" + id }, name + " PPI Viewer")
    ),
    td(
      { class: "myr" },
      a(
        { href: "viewer.html?id=" + id + "&draw=alt" },
        name + " PPI (WebGL alt)"
      )
    ),
    td(
      { class: "myr" },
      a(
        { href: "viewer.html?id=" + id + "&draw=2d" },
        name + " PPI (2D Canvas)"
      )
    ),
    td(
      { class: "myr" },
      a(
        { href: "viewer.html?id=" + id + "&draw=webgpu" },
        name + " PPI (WebGPU)"
      )
    )
  );

function radarsLoaded(d) {
  let c = Object.keys(d).length;
  let r = document.getElementById("radars");

  // Clear previous content
  r.innerHTML = "";

  if (c > 0) {
    van.add(r, div(c + " radar(s) detected"));
    let table = document.createElement("table");
    r.appendChild(table);
    Object.keys(d)
      .sort()
      .forEach(function (v, i) {
        van.add(table, RadarEntry(v, d[v].name));
      });
  } else {
    van.add(r, div({ class: "myr_warning" }, "No radars detected. Waiting for radar beacons..."));

    // Show network requirements help
    van.add(r,
      details({ style: "margin-top: 15px;" },
        summary({ style: "cursor: pointer; font-weight: bold;" }, "Network Configuration Help"),
        div({ style: "margin-top: 10px; padding: 10px; background: #f5f5f5; border-radius: 5px;" },
          p(strong("Furuno DRS (DRS4D-NXT, etc.):")),
          p(NETWORK_REQUIREMENTS.furuno.description),
          p({ style: "font-family: monospace; background: #e0e0e0; padding: 5px;" },
            NETWORK_REQUIREMENTS.furuno.example),

          p({ style: "margin-top: 15px;" }, strong("Navico (Simrad, Lowrance, B&G):")),
          p(NETWORK_REQUIREMENTS.navico.description),

          p({ style: "margin-top: 15px;" }, strong("Raymarine:")),
          p(NETWORK_REQUIREMENTS.raymarine.description),

          p({ style: "margin-top: 15px;" }, strong("Garmin xHD:")),
          p(NETWORK_REQUIREMENTS.garmin.description)
        )
      )
    );
  }

  setTimeout(loadRadars, 15000);
}

function interfacesLoaded(d) {
  if (!d || !d.interfaces) {
    return;
  }

  let c = Object.keys(d.interfaces).length;
  if (c > 0) {
    let r = document.getElementById("interfaces");
    r.innerHTML = "<div>" + c + " interface(s) detected</div><table></table>";
    let table = r.getElementsByTagName("table")[0];

    let brands = ["Interface", ...d.brands];
    let hdr = van.add(table, tr());
    brands.forEach((v) => van.add(hdr, td({ class: "myr" }, v)));

    let interfaces = d.interfaces;
    if (interfaces) {
      console.log("interfaces", interfaces);
      Object.keys(interfaces).forEach(function (v, i) {
        let row = van.add(table, tr());

        van.add(row, td({ class: "myr" }, v));
        if (interfaces[v].status) {
          van.add(
            row,
            td(
              {
                class: "myr_error",
                colspan: d.brands.length,
              },
              interfaces[v].status
            )
          );
        } else {
          d.brands.forEach((b) => {
            let status = interfaces[v].listeners[b];
            let className =
              status == "Listening" || status == "Active" ? "myr" : "myr_error";
            van.add(row, td({ class: className }, status));
          });
        }
      });
    }
  }
}

async function loadRadars() {
  try {
    const radars = await fetchRadars();
    radarsLoaded(radars);
  } catch (err) {
    console.error("Failed to load radars:", err);
    setTimeout(loadRadars, 15000);
  }
}

async function loadInterfaces() {
  try {
    const interfaces = await fetchInterfaces();
    if (interfaces) {
      interfacesLoaded(interfaces);
    } else {
      // Hide interfaces section in SignalK mode
      let r = document.getElementById("interfaces");
      if (r) {
        r.style.display = "none";
      }
    }
  } catch (err) {
    console.error("Failed to load interfaces:", err);
  }
}

window.onload = async function () {
  // Detect mode first
  await detectMode();

  // Load data
  loadRadars();
  loadInterfaces();
};
