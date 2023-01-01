/// Deals with sending the data (x,y,depth,yaw,pitch,roll) to opentrack (https://github.com/opentrack/opentrack) using UDP socket
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use crate::structs::network::SocketNetwork;


impl SocketNetwork {
    pub fn new(ip_arr: (u8, u8, u8, u8), port: u16) -> Self {
        tracing::info!(
            "Sending data to {}.{}.{}.{} on port {port}",
            ip_arr.0,
            ip_arr.1,
            ip_arr.2,
            ip_arr.3
        );

        let address = SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(ip_arr.0, ip_arr.1, ip_arr.2, ip_arr.3)),
            port,
        );

        let socket_network = UdpSocket::bind("{ip_arr.0}.{ip_arr.1}.{ip_arr.2}.{ip_arr.3}:{port}").expect("failed to bind UDP socket");

        Self {
            address,
            socket_network,
        }
    }

    // TODO : Cleaning and possibly removing unsafe code
    pub fn send(&mut self, data: [f32; 6]) {
        let data: [f64; 6] = [
            data[0] as f64,
            data[1] as f64,
            data[2] as f64,
            data[3] as f64,
            data[4] as f64,
            data[5] as f64,
        ];

        // Convert an array to f64 to array of u8
        let out = unsafe {
            std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 4)
        };

        // Send data
        self.socket_network
            .send_to(&out, self.address)
            .expect("failed to send data");
    }
}

#[test]
pub fn test_socket_network() {
    let mut socket_network = SocketNetwork::new((127, 0, 0, 1), 4242);
    socket_network.send([1., 2., 3., 4., 5., 6.])
}
