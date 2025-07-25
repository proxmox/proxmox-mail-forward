include /usr/share/dpkg/pkg-info.mk
include /usr/share/dpkg/architecture.mk

PACKAGE=proxmox-mail-forward
BUILDDIR ?= $(PACKAGE)-$(DEB_VERSION)

DSC=rust-$(PACKAGE)_$(DEB_VERSION_UPSTREAM).dsc
DEB=$(PACKAGE)_$(DEB_VERSION)_$(DEB_HOST_ARCH).deb
DBG_DEB=$(PACKAGE)-dbgsym_$(DEB_VERSION)_$(DEB_HOST_ARCH).deb

DEBS=$(DEB) $(DBG_DEB)

ifeq ($(BUILD_MODE), release)
CARGO_BUILD_ARGS += --release
COMPILEDIR := target/release
else
COMPILEDIR := target/debug
endif

CARGO ?= cargo

$(BUILDDIR):
	rm -rf $@ $@.tmp && mkdir $@.tmp
	cp -a debian Cargo.toml src Makefile .cargo $@.tmp
	rm -f $@.tmp/Cargo.lock
	find $@.tmp/debian -name "*.hint" -delete
	mv $@.tmp $@

.PHONY: deb dsc
$(DEBS): deb
deb: $(BUILDDIR)
	cd $(BUILDDIR); dpkg-buildpackage -b -us -uc
	lintian $(DEBS)

dsc: clean
	$(MAKE) $(DSC)
	lintian $(DSC)

$(DSC): $(BUILDDIR)
	cd $(BUILDDIR); dpkg-buildpackage -S -us -uc -d

sbuild: $(DSC)
	sbuild $<

.PHONY: dinstall
dinstall: $(DEBS)
	dpkg -i $(DEBS)

.PHONY: cargo-build
cargo-build:
	$(CARGO) build $(CARGO_BUILD_ARGS) \
	    --package proxmox-mail-forward \
	    --bin proxmox-mail-forward

install: cargo-build
	$(CARGO) install

.PHONY: upload
upload: $(DEBS)
	tar cf - $(DEBS) | ssh -X repoman@repo.proxmox.com -- upload --product pve,pbs,pdm --dist $(DEB_DISTRIBUTION) --arch $(DEB_HOST_ARCH)

.PHONY: distclean
distclean: clean

.PHONY: clean
clean:
	cargo clean
	rm -rf *.deb *.dsc *.buildinfo *.changes *.build rust-$(PACKAGE)*.tar.* $(PACKAGE)-[0-9]*/
	find . -name '*~' -exec rm {} ';'
