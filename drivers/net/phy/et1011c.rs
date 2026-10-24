// SPDX-License-Identifier: GPL-2.0
// Copyright (C) 2008 Texas Instruments.

//! LSI ET1011C PHY driver
//!
//! Rust port of the C driver `drivers/net/phy/et1011c.c`, deleted in the
//! same change.

use kernel::{
    net::phy::{self, reg::C22, DeviceId, Driver},
    prelude::*,
    sync::atomic::{Atomic, Relaxed},
    uapi,
};

kernel::module_phy_driver! {
    drivers: [PhyEt1011C],
    device_table: [DeviceId::new_with_driver::<PhyEt1011C>()],
    name: "et1011c",
    authors: ["Chaithrika U S"],
    description: "LSI ET1011C PHY driver",
    license: "GPL",
}

/// ET1011C status register (vendor-specific).
const ET1011C_STATUS_REG: C22 = C22::vendor_specific::<0x1a>();
/// ET1011C config register (vendor-specific).
const ET1011C_CONFIG_REG: C22 = C22::vendor_specific::<0x16>();
const ET1011C_SPEED_MASK: u16 = 0x0300;
const ET1011C_GIGABIT_SPEED: u16 = 0x0200;
const ET1011C_TX_FIFO_MASK: u16 = 0x3000;
const ET1011C_TX_FIFO_DEPTH_16: u16 = 0x1000;
const ET1011C_GMII_INTERFACE: u16 = 0x0002;
const ET1011C_SYS_CLK_EN: u16 = 0x01 << 4;

const BMCR_FULLDPLX: u16 = uapi::BMCR_FULLDPLX as u16;
const BMCR_SPEED100: u16 = uapi::BMCR_SPEED100 as u16;
const BMCR_SPEED1000: u16 = uapi::BMCR_SPEED1000 as u16;
const BMCR_ANENABLE: u16 = uapi::BMCR_ANENABLE as u16;
const BMCR_RESET: u16 = uapi::BMCR_RESET as u16;

/// Last link speed seen by [`PhyEt1011C::read_status`]; the vendor-specific
/// registers are only reconfigured when the speed has changed (the C driver
/// kept this in a `static int speed`).
static LAST_SPEED: Atomic<u32> = Atomic::new(0);

struct PhyEt1011C;

#[vtable]
impl Driver for PhyEt1011C {
    const NAME: &'static CStr = c"ET1011C";
    const PHY_DEVICE_ID: DeviceId = DeviceId::new_with_custom_mask(0x0282f014, 0xfffffff0);

    fn config_aneg(dev: &mut phy::Device) -> Result {
        let ctl = dev.read(C22::BMCR)?;
        // First clear the PHY: clear the full-duplex, 100M and 1000M and the
        // auto-negotiation-enable bits, then issue a software reset.
        // As in the C driver, the result of the write is not checked.
        let new_ctl =
            ctl & !(BMCR_FULLDPLX | BMCR_SPEED100 | BMCR_SPEED1000 | BMCR_ANENABLE) | BMCR_RESET;
        let _ = dev.write(C22::BMCR, new_ctl);

        dev.genphy_config_aneg()
    }

    fn read_status(dev: &mut phy::Device) -> Result<u16> {
        let status = dev.genphy_read_status::<C22>()?;

        // Reconfigure the vendor-specific registers when the link speed has
        // changed since the last call.
        let cur_speed = dev.speed();
        if LAST_SPEED.load(Relaxed) != cur_speed {
            LAST_SPEED.store(cur_speed, Relaxed);

            // As in the C driver, the results of the register accesses are
            // not checked here; only reconfigure when the PHY reports a
            // gigabit link.
            if let Ok(val) = dev.read(ET1011C_STATUS_REG) {
                if (val & ET1011C_SPEED_MASK) == ET1011C_GIGABIT_SPEED {
                    if let Ok(val) = dev.read(ET1011C_CONFIG_REG) {
                        let config = (val & !ET1011C_TX_FIFO_MASK)
                            | ET1011C_GMII_INTERFACE
                            | ET1011C_SYS_CLK_EN
                            | ET1011C_TX_FIFO_DEPTH_16;
                        let _ = dev.write(ET1011C_CONFIG_REG, config);
                    }
                }
            }
        }

        Ok(status)
    }
}
