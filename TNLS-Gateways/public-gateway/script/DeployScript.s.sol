// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.23;

import "forge-std/Test.sol";
import "forge-std/Vm.sol";
import "forge-std/console2.sol";
import "forge-std/Script.sol";
import {Gateway} from "../src/Gateway.sol";
import {RandomnessReciever} from "../src/RandomnessReciever.sol";

contract DeployScript is Script {
    function setUp() public {}

    Gateway gatewayAddress;
    RandomnessReciever randomnessAddress;
    
    uint256 privKey = vm.envUint("ETH_PRIVATE_KEY");
    address deployer = vm.rememberKey(privKey);

        /// @notice Get the encoded hash of the inputs for signing
    /// @param _routeInput Route name
    /// @param _verificationAddressInput Address corresponding to the route
    function getRouteHash(string memory _routeInput, address _verificationAddressInput) public pure returns (bytes32) {
        return keccak256(abi.encode(_routeInput, _verificationAddressInput));
    }

        /// @notice Hashes the encoded message hash
    /// @param _messageHash the message hash
    function getEthSignedMessageHash(bytes32 _messageHash) public pure returns (bytes32) {
        /*
        Signature is produced by signing a keccak256 hash with the following format:
        "\x19Ethereum Signed Message\n" + len(msg) + msg
        */
        return keccak256(abi.encodePacked("\x19Ethereum Signed Message:\n32", _messageHash));
    }

    function run() public {
        vm.startBroadcast();

        gatewayAddress = new Gateway();
        randomnessAddress = new RandomnessReciever();

        console2.logAddress(address(gatewayAddress));
        console2.logAddress(deployer);

        randomnessAddress.setGatewayAddress(address(gatewayAddress));
        // Initialize master verification Address
        gatewayAddress.setMasterVerificationAddress(deployer);
        gatewayAddress.setContractAddressAndCodeHash("secret1u3zsgg680f0fmjv7w6qkhpe6sh78wngzevvlvd", 
        "866d57146b2d9cccde295b57d84f22995568ffb5dc31a795e73f351bbfb19dfb");
        address verificationAddress = 0xBC0951b2f849020027921e7286aE134C8e159203;
        gatewayAddress.setChainidentifier("ethereum");
        /// ------ Update Routes Param Setup ------- ///

        string memory route = "secret";
        //address verificationAddress = vm.envAddress("SECRET_GATEWAY_ETH_ADDRESS");

        // Update the route with with masterVerificationKey signature
        bytes32 routeHash = getRouteHash(route, verificationAddress);
        bytes32 ethSignedMessageHash = getEthSignedMessageHash(routeHash);

        (uint8 v, bytes32 r, bytes32 s) = vm.sign(privKey, ethSignedMessageHash);
        bytes memory sig = abi.encodePacked(r, s, v);

        gatewayAddress.updateRoute(route, verificationAddress, sig);

        vm.stopBroadcast();
    }
}
