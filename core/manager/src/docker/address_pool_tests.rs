//! Pins down where the networks Thoric makes get their addresses: what `DESKTOP_ADDRESS_POOL`
//! accepts, the first subnet of it that no Docker network overlaps, and the errors that name the
//! setting once the range, or Docker's own pools, are full.
use super::*;

fn subnet(text: &str) -> Subnet {
    Subnet::parse(text).unwrap()
}

#[test]
fn the_pool_is_a_range_of_its_own_unless_docker_chooses() {
    assert_eq!(address_pool(None).unwrap(), Some(DEFAULT_POOL));
    assert_eq!(address_pool(Some("")).unwrap(), Some(DEFAULT_POOL));
    assert_eq!(DEFAULT_POOL.to_string(), "10.213.0.0/16");
    assert_eq!(address_pool(Some("docker")).unwrap(), None);
    for range in [
        "10.0.0.0/8",
        "172.16.0.0/24",
        "100.64.0.0/10",
        "10.213.4.0/22",
    ] {
        assert_eq!(
            address_pool(Some(range))
                .unwrap()
                .map(|pool| pool.to_string()),
            Some(range.to_owned())
        );
    }
    for wrong in [
        // Not its first address, too small for a group's network, wider than a /8.
        "10.213.0.1/16",
        "10.213.0.0/25",
        "10.0.0.0/7",
        // Not an IPv4 range at all.
        "fd00::/64",
        "10.213.0.0",
        "10.213.0.0/",
        "10.213.0.0/16 ",
        "Docker",
        "none",
        // Addresses no network takes.
        "127.0.0.0/16",
        "0.0.0.0/8",
        "224.0.0.0/8",
        "240.0.0.0/8",
    ] {
        let error = address_pool(Some(wrong)).unwrap_err();
        assert!(
            format!("{error:#}").starts_with("DESKTOP_ADDRESS_POOL must be"),
            "{wrong}: {error:#}"
        );
    }
}

#[test]
fn a_network_takes_the_first_subnet_no_docker_network_overlaps() {
    let pool = DEFAULT_POOL;
    let free = |prefix, taken: &[&str]| {
        let taken = taken.iter().map(|text| subnet(text)).collect::<Vec<_>>();
        free_subnet(pool, prefix, &taken).map(|subnet| subnet.to_string())
    };
    let desktop = |taken: &[&str]| free(DESKTOP_PREFIX, taken);
    assert_eq!(desktop(&[]).as_deref(), Some("10.213.0.0/28"));
    // Docker's own networks lie outside the range.
    assert_eq!(
        desktop(&["172.17.0.0/16", "192.168.0.0/20", "10.8.0.0/16"]).as_deref(),
        Some("10.213.0.0/28")
    );
    assert_eq!(
        desktop(&["10.213.0.0/28", "10.213.0.16/28"]).as_deref(),
        Some("10.213.0.32/28")
    );
    // A group's network starts at the next /24, and a desktop's fills the gap before it.
    assert_eq!(
        free(GROUP_PREFIX, &["10.213.0.0/28", "10.213.0.16/28"]).as_deref(),
        Some("10.213.1.0/24")
    );
    assert_eq!(
        desktop(&["10.213.0.0/28", "10.213.1.0/24"]).as_deref(),
        Some("10.213.0.16/28")
    );
    // A network someone made in the range by hand, as large as it is.
    assert_eq!(
        desktop(&["10.213.0.0/20"]).as_deref(),
        Some("10.213.16.0/28")
    );
    assert_eq!(
        desktop(&["10.213.0.64/26", "10.213.0.0/27"]).as_deref(),
        Some("10.213.0.32/28")
    );
    // One that holds the whole range leaves nothing.
    assert_eq!(desktop(&["10.0.0.0/8"]), None);
    assert_eq!(desktop(&["10.213.0.0/16"]), None);

    let small = subnet("10.213.0.0/24");
    let all = (0..16)
        .map(|block| subnet(&format!("10.213.0.{}/28", block * 16)))
        .collect::<Vec<_>>();
    assert_eq!(free_subnet(small, DESKTOP_PREFIX, &all), None);
    assert_eq!(
        free_subnet(small, DESKTOP_PREFIX, &all[..15]).map(|s| s.to_string()),
        Some("10.213.0.240/28".to_owned())
    );
    // A group's network takes the whole of a /24 range, or nothing.
    assert_eq!(free_subnet(small, GROUP_PREFIX, &[]), Some(small));
    assert_eq!(free_subnet(small, GROUP_PREFIX, &all[3..4]), None);
    // The last subnet of the address space.
    let top = subnet("223.255.255.0/24");
    assert_eq!(
        free_subnet(top, DESKTOP_PREFIX, &[subnet("223.255.255.0/25")]).map(|s| s.to_string()),
        Some("223.255.255.128/28".to_owned())
    );
    assert_eq!(free_subnet(top, DESKTOP_PREFIX, &[top]), None);
}

#[test]
fn docker_s_listing_reads_as_its_ipv4_subnets() {
    let listed =
        listed_subnets("172.17.0.0/16 \n\n10.213.0.0/28 fd00:7468::/64 \n172.30.0.5/16 \n");
    assert_eq!(
        listed.iter().map(Subnet::to_string).collect::<Vec<_>>(),
        ["172.17.0.0/16", "10.213.0.0/28", "172.30.0.0/16"]
    );
    assert!(listed_subnets("").is_empty());
}

#[test]
fn a_full_pool_names_the_setting_to_change() {
    let full = pool_full(DEFAULT_POOL, DESKTOP_PREFIX, "thoric-x-egress");
    assert!(
        full.starts_with("DESKTOP_ADDRESS_POOL (10.213.0.0/16) has no /28 left"),
        "{full}"
    );
    assert!(
        full.contains("thoric-x-egress")
            && full.contains("set DESKTOP_ADDRESS_POOL to a larger range"),
        "{full}"
    );
    let exhausted = anyhow::anyhow!(
        "Docker: Error response from daemon: all predefined address pools have been fully subnetted"
    );
    let explained = format!("{:#}", docker_pools(exhausted));
    assert!(
        explained.starts_with("Docker's default address pools are full; set DESKTOP_ADDRESS_POOL to a range of Thoric's own, such as 10.213.0.0/16"),
        "{explained}"
    );
    assert!(explained.ends_with("fully subnetted"), "{explained}");
    let other = format!(
        "{:#}",
        docker_pools(anyhow::anyhow!(
            "Docker: network with name x already exists"
        ))
    );
    assert_eq!(other, "Docker: network with name x already exists");
}

/// A Docker CLI that lists the networks of `ids` with the subnets of `subnets`, refuses
/// 10.213.0.16/28 and every subnet for `thoric-refused` as overlapping, has full pools for
/// `thoric-full`, makes every other network, and logs every call.
#[cfg(unix)]
fn fake_docker(pool: Option<Subnet>) -> (std::sync::Arc<App>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "thoric-address-pool-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ids"), "a1\nb2\n").unwrap();
    std::fs::write(dir.join("subnets"), "172.17.0.0/16 \n10.213.0.0/28 \n").unwrap();
    let docker = dir.join("docker");
    let script = r#"#!/bin/sh
cd 'DIR'
echo "$*" >> calls
overlap() { echo 'Error response from daemon: invalid pool request: Pool overlaps with other one on this address space' >&2; exit 1; }
case "$*" in
  'network ls --quiet --no-trunc') cat ids ;;
  'network inspect --format '*) cat subnets ;;
  *'--subnet 10.213.0.16/28 '*) overlap ;;
  *'--subnet '*' thoric-refused') overlap ;;
  *' thoric-full') echo 'Error response from daemon: all predefined address pools have been fully subnetted' >&2; exit 1 ;;
  'network create '*) eval "name=\${$#}"; echo "$name" ;;
  'network inspect '*) echo "[{\"Id\":\"$3\"}]" ;;
  *) exit 1 ;;
esac
"#;
    crate::test_support::write_script(&docker, &script.replace("DIR", &dir.display().to_string()));
    let mut app = crate::test_support::app(false);
    let config = &mut std::sync::Arc::get_mut(&mut app).unwrap().config;
    config.docker_bin = docker.display().to_string();
    config.address_pool = pool;
    (app, dir)
}

#[cfg(unix)]
#[tokio::test]
async fn networks_take_subnets_of_the_pool_past_the_ones_docker_uses() {
    let (app, dir) = fake_docker(Some(DEFAULT_POOL));
    let labels = ["thoric.managed=true".to_owned()];
    let calls = || {
        let calls = std::fs::read_to_string(dir.join("calls")).unwrap_or_default();
        std::fs::write(dir.join("calls"), "").unwrap();
        calls
    };
    // 10.213.0.0/28 is taken, and Docker refuses the next as overlapping a network it has.
    let made = app
        .make_network(
            "thoric-x-egress",
            &labels,
            Reach::Internet,
            None,
            DESKTOP_PREFIX,
        )
        .await
        .unwrap();
    assert_eq!(made["Id"], "thoric-x-egress");
    let made = calls();
    assert!(
        made.contains(
            "network create --label thoric.managed=true --subnet 10.213.0.16/28 thoric-x-egress\n"
        ),
        "{made}"
    );
    assert!(
        made.ends_with(
            "network create --label thoric.managed=true --subnet 10.213.0.32/28 thoric-x-egress\nnetwork inspect thoric-x-egress\n"
        ),
        "{made}"
    );
    assert!(
        made.contains("network inspect --format {{range .IPAM.Config}}{{.Subnet}} {{end}} a1 b2\n"),
        "{made}"
    );

    app.make_network(
        "thoric-group-build",
        &labels,
        Reach::Nothing,
        None,
        GROUP_PREFIX,
    )
    .await
    .unwrap();
    assert!(
        calls().contains("--subnet 10.213.1.0/24 thoric-group-build\n"),
        "a group's network takes a /24"
    );

    // A private network made again keeps the subnet its desktop was told, also outside the pool.
    app.make_network(
        "thoric-x-private",
        &labels,
        Reach::Nothing,
        Some("172.18.0.0/16"),
        DESKTOP_PREFIX,
    )
    .await
    .unwrap();
    let kept = calls();
    assert!(!kept.contains("network ls"), "{kept}");
    assert!(
        kept.contains("--subnet 172.18.0.0/16 thoric-x-private\n"),
        "{kept}"
    );

    let refused = app
        .make_network(
            "thoric-refused",
            &labels,
            Reach::Internet,
            None,
            DESKTOP_PREFIX,
        )
        .await
        .unwrap_err();
    assert!(
        format!("{refused:#}").starts_with(&format!(
            "Docker refused {ATTEMPTS} subnets of DESKTOP_ADDRESS_POOL (10.213.0.0/16)"
        )),
        "{refused:#}"
    );
    assert_eq!(calls().matches("network create").count(), ATTEMPTS);
    std::fs::remove_dir_all(&dir).unwrap();

    // Nothing of a small range is left for a group once a desktop's network is in it.
    let (app, dir) = fake_docker(Some(subnet("10.213.0.0/24")));
    let full = app
        .make_network(
            "thoric-group-build",
            &labels,
            Reach::Nothing,
            None,
            GROUP_PREFIX,
        )
        .await
        .unwrap_err();
    assert!(
        format!("{full:#}").starts_with("DESKTOP_ADDRESS_POOL (10.213.0.0/24) has no /24 left"),
        "{full:#}"
    );
    let asked = std::fs::read_to_string(dir.join("calls")).unwrap();
    assert!(!asked.contains("network create"), "{asked}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn docker_s_own_pools_choose_when_the_setting_says_docker() {
    let (app, dir) = fake_docker(None);
    let labels = ["thoric.managed=true".to_owned()];
    app.make_network(
        "thoric-x-egress",
        &labels,
        Reach::Internet,
        None,
        DESKTOP_PREFIX,
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("calls")).unwrap(),
        "network create --label thoric.managed=true thoric-x-egress\nnetwork inspect thoric-x-egress\n"
    );
    let full = app
        .make_network(
            "thoric-full",
            &labels,
            Reach::Internet,
            None,
            DESKTOP_PREFIX,
        )
        .await
        .unwrap_err();
    assert!(
        format!("{full:#}")
            .starts_with("Docker's default address pools are full; set DESKTOP_ADDRESS_POOL"),
        "{full:#}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
